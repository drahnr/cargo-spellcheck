//! Suggestions are individual correctable items where items are either words, punctuation
//! or even complete sentences.
//!
//! ```raw
//! error[spellcheck]: Spelling
//! --> src/main.rs:138:16
//!     |
//! 138 | /// Thisf module is for easing the pain with printing text in the terminal.
//!     |     ^^^^^
//!     |     - The word "Thisf" is not in our dictionary. If you are sure this spelling is correcformatter,
//!     |     - you can add it to your personal dictionary to prevent future alerts.
//! ```

use crossterm::terminal::size;
use std::path::{Path, PathBuf};

use crate::Span;
use crate::TrimmedLiteralRef;

use crate::literalset::Range;
use enumflags2::BitFlags;
use log::error;

/// Bitflag of available checkers by compilation / configuration.
#[derive(Debug, Clone, Copy, BitFlags, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum Detector {
    Hunspell = 0b0001,
    LanguageTool = 0b0010,
}

pub fn get_terminal_size() -> usize {
    use super::*;
    const DEFAULT_TERMINAL_SIZE: usize = 80;
    match size() {
        Ok((terminal_size, _)) => terminal_size as usize,
        Err(_) => {
            warn!(
                "Unable to get terminal size. Use default: {}",
                DEFAULT_TERMINAL_SIZE
            );
            DEFAULT_TERMINAL_SIZE
        }
    }
}
// impl
// // TODO use this to display included compiled backends
// fn list_available() {
//     match detector {
//         Detector::Hunspell => cfg!(feature="hunspell"),
//         Detector::LanguageTool => cfg!(feature="languagetool"),
//     }
// }

use std::fmt;

impl fmt::Display for Detector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::LanguageTool => "LanguageTool",
            Self::Hunspell => "Hunspell",
        })
    }
}

// For long lines, literal will be trimmed to display in one terminal line
// Misspelled words that are too long shall also be ellipsized
pub fn convert_long_statements_to_short<'s>(
    terminal_size: usize,
    marker_range_relative: Range,
    marker_size: &mut usize,
    literal: TrimmedLiteralRef<'s>,
    offset: &mut usize,
    indent: usize,
    padding_till_literal_start: usize,
) -> String {
    //
    // The paddings give some space for the ` {} ...` and extra indentation and formatting:
    //
    //|---------------------------------------------------------------------------------------| terminal_size
    //|-------| padding_till_literal_start = indent (3+line_number_digit_count) + 2 white spaces = 7usize, for this case.
    //     |------| offset = PADDING_OFFSET; 3 chars for `...` and 2 white spaces more added in the formatting.
    //
    //   --> /home/tmhdev/Documents/cargo-spellcheck/src/suggestion.rs:62
    //    |
    // 62 |  ... Reasn of food, what's up with pie? There's strawberry pie, apple, pumpkin ...
    //    |      ^^^^^^
    //    | - there, Cherie, thither, or tither
    //    |
    //    |   Possible spelling mistake found.
    //
    const PADDING_OFFSET: usize = 5;
    const TOO_LONG_WORD: usize = 20;
    const DISPLAYED_LONG_WORD: usize = 4;
    const PADDING_AROUND_LONG_LINES: usize = 10;

    // We will be using ranges to help doing the fitting:
    //
    // |----------------------------------literal_word----------------------------------|
    // |----------------------|---------misspelled_word---------|-----------------------|
    // |-----left_context-----|---start_word----|----end_word---|-----right_context-----|
    //
    // Obs: paddings are not being considered in the illustration, but info is above.
    let mut range_left_context = Range {
        start: 0usize,
        end: marker_range_relative.start,
    };
    let mut range_right_context = Range {
        start: marker_range_relative.end,
        end: literal.as_str().chars().count(),
    };
    let mut range_start_word = Range {
        start: marker_range_relative.start,
        end: marker_range_relative.start,
    };
    let mut range_end_word = Range {
        start: marker_range_relative.end,
        end: marker_range_relative.end,
    };
    // the line being analysed can affect how the indentation is done
    // this values is dynamically calculated according to the line number
    let mut misspelled_word = format!(
        "{}",
        // Exactly range to use sub() and have access to the misspelled word
        // without extra spaces or punctuation around
        literal.sub(Range {
            start: (marker_range_relative.start).saturating_sub(1),
            end: (marker_range_relative.end).saturating_sub(1)
        })
    );
    // Check words that are considered too long; Word will be formatted for fitting
    if *marker_size > TOO_LONG_WORD {
        range_start_word = Range {
            start: marker_range_relative.start - 1,
            end: range_start_word.start + DISPLAYED_LONG_WORD,
        };
        range_end_word = Range {
            start: marker_range_relative
                .end //non inclusive
                .saturating_sub(DISPLAYED_LONG_WORD),
            end: marker_range_relative.end - 1,
        };

        //  too long word will be shorter as it follows:
        //    4 chars |----|  ... |---| 3 chars
        //                ther...eee, for therieeeeeeeeeeeeeeee
        //
        misspelled_word = format!(
            "{}...{}",
            literal.sub(range_start_word),
            literal.sub(range_end_word)
        );
        *marker_size = misspelled_word.chars().count();
    }
    // right context has enough info to fill the terminal
    // |-----misspelled_word-----|--------right_context---------|
    //
    // Attempt to fit the misspelled word in the beginning followed by info.
    if range_right_context.len() >= terminal_size {
        range_right_context = Range {
            start: marker_range_relative.end - 1, //char right after the end of the word and it shall be included, white space.
            end: marker_range_relative.end
                + (terminal_size.saturating_sub(
                    misspelled_word.chars().count()
                        + PADDING_AROUND_LONG_LINES
                        + padding_till_literal_start
                        + 1,
                )),
        };
        // Left range will not be used in this case
        range_left_context = Range {
            start: 0usize,
            end: 0usize,
        };
        *offset = PADDING_OFFSET;
    }
    // left context has enough info to fill the terminal
    // |---------left_context---------|-----misspelled_word-----|
    //
    // Attempt to fit the misspelled word with left context info
    else if range_left_context.len() > terminal_size {
        range_left_context = Range {
            start: marker_range_relative
                .start
                .saturating_sub(terminal_size.saturating_sub(
                    misspelled_word.chars().count()
                        + PADDING_AROUND_LONG_LINES
                        + padding_till_literal_start,
                )),
            end: marker_range_relative.start - 1,
        };
        // Right range will not be used
        range_right_context = Range {
            start: 0usize,
            end: 0usize,
        };
        *offset = range_left_context.len() + PADDING_OFFSET;
    }
    // information will be shown in both sides of the `misspelled_word`
    // |--left_context--|----misspelled_word---|--right_context--|
    //
    // Attempt to fit the misspelled word in the middle with info in th left and right of it
    else {
        let context = (terminal_size.saturating_sub(
            misspelled_word.chars().count()
                + padding_till_literal_start
                + PADDING_AROUND_LONG_LINES,
        )) / 2;
        range_left_context = Range {
            start: range_left_context.end.saturating_sub(context),
            end: marker_range_relative.start - 1, //before the word starts
        };
        range_right_context = Range {
            start: marker_range_relative.end - 1,
            end: range_right_context.start + context,
        };
        *offset = range_left_context.len() + PADDING_OFFSET;
    }
    // Formatting itself added white spaces and punctuation to do the fitting to be considered:
    //
    //     |------ info ----| => PADDING_AROUND_LONG_LINES = 10 usize
    // format!(
    //     "  ... {}{}{} ...",
    //     self.literal.sub(range_left_context),
    //     misspelled_word,
    //     self.literal.sub(range_right_context)
    // )
    format!(
        "  ... {}{}{} ...",
        literal.sub(range_left_context),
        misspelled_word,
        literal.sub(range_right_context)
    )
}

/// A suggestion for certain offending span.
#[derive(Clone, Hash, PartialEq, Eq)]
pub struct Suggestion<'s> {
    /// Which checker suggested the change.
    pub detector: Detector,
    /// Reference to the file location the `span` and `literal` relate to.
    pub path: PathBuf,
    /// Literal we are referencing.
    pub literal: TrimmedLiteralRef<'s>,
    /// The span (absolute!) of where it is supposed to be used.
    pub span: Span,
    /// Fix suggestions, might be words or the full sentence.
    pub replacements: Vec<String>,
    /// Descriptive reason for the suggestion.
    pub description: Option<String>,
}

impl<'s> fmt::Display for Suggestion<'s> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use console::Style;

        let highlight = Style::new().bold().white();
        let error = Style::new().bold().red();
        let arrow_marker = Style::new().blue();
        let context_marker = Style::new().bold().blue();
        let fix = Style::new().green();
        let help = Style::new().yellow().bold();

        let line_number_digit_count = self.span.start.line.to_string().len();
        let indent = 3 + line_number_digit_count;

        error.apply_to("error").fmt(formatter)?;
        highlight
            .apply_to(format!(": spellcheck({})", &self.detector))
            .fmt(formatter)?;
        formatter.write_str("\n")?;

        arrow_marker
            .apply_to(format!("{:>width$}", "-->", width = indent + 1))
            .fmt(formatter)?;

        writeln!(
            formatter,
            " {path}:{line}",
            path = self.path.display(),
            line = self.span.start.line
        )?;
        context_marker
            .apply_to(format!("{:>width$}", "|", width = indent))
            .fmt(formatter)?;
        formatter.write_str("\n")?;
        context_marker
            .apply_to(format!(
                "{:>width$} |",
                self.span.start.line,
                width = indent - 2,
            ))
            .fmt(formatter)?;

        // underline the relevant part with ^^^^^

        // @todo this needs some more thought once multiline comments pop up
        let mut marker_size = if self.span.end.line == self.span.start.line {
            // column bounds are inclusive, so for a correct length we need to add + 1
            self.span.end.column.saturating_sub(self.span.start.column) + 1
        } else {
            self.literal.len().saturating_sub(self.span.start.column)
        };

        let literal_span: Span = Span::from(self.literal.as_ref().literal.span());
        let marker_range_relative: Range = self.span.relative_to(literal_span).expect("Must be ok");

        // if the offset starts from 0, we still want to continue if the length
        // of the marker is at least length 1

        let mut offset = if self.literal.pre() <= marker_range_relative.start {
            marker_range_relative.start - self.literal.pre()
        } else {
            error!("Reducing marker length! Please report a BUG!");
            // reduce the marker size
            marker_size -= marker_range_relative.start;
            marker_size -= self.literal.pre();
            0
        };

        let terminal_size: usize = get_terminal_size();
        // the line being analysed can affect how the indentation is done
        // this values is dynamically calculated for each line where the documentation
        let padding_till_literal_start = indent + 2; // 2 extra spaces are considered for starting the literal already

        // Check whether the statement is too long for the remaining space left of the terminal size
        // and if it is, we shall do the fitting
        if self.literal.as_str().chars().count() + padding_till_literal_start > terminal_size {
            let mut misspelled_word = format!(
                "{}",
                // Exactly range to use sub() and have access to the misspelled word
                // without extra spaces or punctuation around
                self.literal.sub(Range {
                    start: (marker_range_relative.start).saturating_sub(1),
                    end: (marker_range_relative.end).saturating_sub(1)
                })
            );
            let formatted_literal: String = convert_long_statements_to_short(
                terminal_size,
                marker_range_relative,
                &mut marker_size,
                self.literal,
                &mut offset,
                indent,
                padding_till_literal_start,
            );
            writeln!(formatter, "{}", &formatted_literal)?;

        // literal is smaller than terminal size and it can be fully displayed
        } else {
            writeln!(formatter, " {}", self.literal.as_str())?;
        }

        if marker_size > 0 {
            context_marker
                .apply_to(format!("{:>width$}", "|", width = indent))
                .fmt(formatter)?;
            help.apply_to(format!(" {:>offset$}", "", offset = offset))
                .fmt(formatter)?;
            help.apply_to(format!("{:^>size$}", "", size = marker_size))
                .fmt(formatter)?;
            formatter.write_str("\n")?;
            log::trace!(
                "marker_size={} [{}|{}|{}] literal {{ {:?} .. {:?} }} >> {:?} <<",
                marker_size,
                self.literal.pre(),
                self.literal.len(),
                self.literal.post(),
                self.span.start,
                self.span.end,
                self,
            );
        } else {
            log::warn!(
                "marker_size={} [{}|{}|{}] literal {{ {:?} .. {:?} }} >> {:?} <<",
                marker_size,
                self.literal.pre(),
                self.literal.len(),
                self.literal.post(),
                self.span.start,
                self.span.end,
                self,
            );
        }

        context_marker
            .apply_to(format!("{:>width$}", "|", width = indent))
            .fmt(formatter)?;

        let replacement = match self.replacements.len() {
            0 => String::new(),
            1 => format!(" - {}", fix.apply_to(&self.replacements[0])),
            2 => format!(
                " - {} or {}",
                fix.apply_to(&self.replacements[0]).to_string(),
                fix.apply_to(&self.replacements[1]).to_string()
            ),
            n if (n < 7) => {
                let last = fix.apply_to(&self.replacements[n - 1]).to_string();
                let joined = self.replacements[..n - 1]
                    .iter()
                    .map(|x| fix.apply_to(x).to_string())
                    .collect::<Vec<String>>()
                    .as_slice()
                    .join(", ");
                format!(" - {}, or {}", joined, last)
            }
            _n => {
                let joined = self.replacements[..=6]
                    .iter()
                    .map(|x| fix.apply_to(x).to_string())
                    .collect::<Vec<String>>()
                    .as_slice()
                    .join(", ");

                let remaining = self.replacements.len() - 6;
                let remaining = fix.apply_to(format!("{}", remaining)).to_string();
                format!(" - {}, or one of {} others", joined, remaining)
            }
        };

        error.apply_to(replacement).fmt(formatter)?;
        formatter.write_str("\n")?;

        context_marker
            .apply_to(format!("{:>width$}", "|\n", width = indent + 1))
            .fmt(formatter)?;

        context_marker
            .apply_to(format!("{:>width$}", "|", width = indent))
            .fmt(formatter)?;
        if let Some(ref description) = self.description {
            writeln!(formatter, "   {}", description)?;
        }

        context_marker
            .apply_to(format!("{:>width$}", "|\n", width = indent + 1))
            .fmt(formatter)
    }
}

impl<'s> fmt::Debug for Suggestion<'s> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use crate::literalset::TrimmedLiteralDisplay;

        let printable = TrimmedLiteralDisplay::from((
            self.literal,
            self.span
                .relative_to(self.literal.as_ref().literal.span())
                .expect("Must be on the same line"),
        ));
        write!(formatter, "({}, {:?})", &printable, printable.1)
    }
}

/// A set of suggestions across multiple files, clustered per file
#[derive(Debug, Clone)]
pub struct SuggestionSet<'s> {
    per_file: indexmap::IndexMap<PathBuf, Vec<Suggestion<'s>>>,
}

impl<'s> SuggestionSet<'s> {
    pub fn new() -> Self {
        Self {
            per_file: indexmap::IndexMap::with_capacity(64),
        }
    }

    pub fn iter<'a>(
        &'a self,
    ) -> impl DoubleEndedIterator<Item = (&'a PathBuf, &'a Vec<Suggestion<'s>>)> {
        self.per_file.iter()
    }

    pub fn add(&mut self, path: PathBuf, suggestion: Suggestion<'s>) {
        self.per_file
            .entry(path)
            .or_insert_with(|| Vec::with_capacity(1))
            .push(suggestion);
    }

    pub fn append(&mut self, path: PathBuf, suggestions: &[Suggestion<'s>]) {
        self.per_file
            .entry(path)
            .or_insert_with(|| Vec::with_capacity(32))
            .extend_from_slice(suggestions);
    }

    pub fn extend<I>(&mut self, path: PathBuf, suggestions: I)
    where
        I: IntoIterator<Item = Suggestion<'s>>,
    {
        let v: &mut Vec<Suggestion<'s>> = self
            .per_file
            .entry(path)
            .or_insert_with(|| Vec::with_capacity(32));
        v.extend(suggestions.into_iter());
    }

    /// Obtain an accessor `Entry` for the given `path`
    pub fn entry(&mut self, path: PathBuf) -> indexmap::map::Entry<PathBuf, Vec<Suggestion<'s>>> {
        self.per_file.entry(path)
    }

    /// Iterate over all files by reference
    pub fn files<'i, 'a>(&'a mut self) -> impl DoubleEndedIterator<Item = &'i Path>
    where
        's: 'i,
        'a: 'i,
    {
        self.per_file.keys().map(|p| p.as_path())
    }

    /// Iterate over all references given a path
    ///
    /// panics if there is no such path
    pub fn suggestions<'a>(
        &'a self,
        path: &Path,
    ) -> impl DoubleEndedIterator<Item = &'a Suggestion<'s>>
    where
        'a: 's,
    {
        if let Some(ref suggestions) = self.per_file.get(path) {
            suggestions.iter()
        } else {
            panic!("Path must exist")
        }
        // intermediate does not live long enough
        // .map(|suggestions: &'s Vec<Suggestion<'s>>| -> std::slice::Iter<'a, Suggestion<'s>> {
        //     (suggestions).into_iter()
        // } ).iter().flatten()
    }

    /// Join two sets
    ///
    /// Merges multiple keys into one.
    pub fn join<I>(&mut self, other: I)
    where
        I: IntoIterator<Item = (PathBuf, Vec<Suggestion<'s>>)>,
    {
        other.into_iter().for_each(|(path, suggestions)| {
            self.entry(path)
                .or_insert_with(|| Vec::with_capacity(suggestions.len()))
                .extend_from_slice(suggestions.as_slice())
        })
    }

    /// Obtain the number of items in the set
    #[inline]
    pub fn len(&self) -> usize {
        self.per_file.len()
    }

    /// Count the number of suggestions accress file in total
    pub fn count(&self) -> usize {
        self.per_file.iter().map(|(_path, vec)| vec.len()).sum()
    }
}

impl<'s> IntoIterator for SuggestionSet<'s> {
    type Item = (PathBuf, Vec<Suggestion<'s>>);
    type IntoIter = indexmap::map::IntoIter<PathBuf, Vec<Suggestion<'s>>>;
    fn into_iter(self) -> Self::IntoIter {
        self.per_file.into_iter()
    }
}

impl<'s> IntoIterator for &'s SuggestionSet<'s> {
    type Item = (&'s PathBuf, &'s Vec<Suggestion<'s>>);
    type IntoIter = indexmap::map::Iter<'s, PathBuf, Vec<Suggestion<'s>>>;
    fn into_iter(self) -> Self::IntoIter {
        self.per_file.iter()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::TrimmedLiteral;

    #[test]
    fn convert_long_statements_to_short_test() {
        const CONTENT_LONG_LITERAL_LONG_WORD: &'static str = "Speakiiiiiinnnnnnnnnnnnnnngggggggg of food, what's up with pie? There's strawberry pie, apple, \
pumpkin and so many others, but there is no grape pie! I know. I'm just as upset about this unfortunate \
lack of development in the pie division. Think about it. Grapes are used to make jelly, jam, juice and raisins. \
What makes them undesirable for pie? Would they dry into raisins? Couldn't you just stick some jelly in a \
piecrust and bake it? It just doesn't make any sense. Another thing that bothers me is organ grinders. \
You know, the foreign guys with the bellhop hats and the little music thingy and the cute little monkey with the \
bellhop hat who collects the money? Okay. They're basically begging on the street. How did they ever afford an \
organ-thingy? Wouldn't it make more sense to get a kazoo, if you're broke? And if they're so poor, what possessed \
them to buy a monkey?
";
        let test_word = TrimmedLiteral {
            literal: proc_macro2::Literal::string(CONTENT_LONG_LITERAL_LONG_WORD),
            rendered: CONTENT_LONG_LITERAL_LONG_WORD.to_owned(),
            pre: 0usize,
            post: 0usize,
            len: CONTENT_LONG_LITERAL_LONG_WORD.len(),
        };

        let test_word_ref = TrimmedLiteralRef {
            reference: &test_word,
        };
        let terminal_size = 80;
        let marker_range_relative = Range {
            start: 1usize,
            end: 35usize,
        };
        let mut marker_size = 34;
        let mut offset: usize = 0;
        let indent: usize = 5;
        let padding_till_literal_start = 7;
        let res: String = convert_long_statements_to_short(
            terminal_size,
            marker_range_relative,
            &mut marker_size,
            test_word_ref,
            &mut offset,
            indent,
            padding_till_literal_start,
        );
        assert_eq!(
            res,
            "  ... Speak...ggg of food, what's up with pie? There's strawberry pie ..."
        );
        assert_eq!(
            res.chars().count(),
            terminal_size.saturating_sub(padding_till_literal_start)
        );
        assert_eq!(offset, 5usize);

        let res: String = String::from("Speakingi of food, what's up with pie? There's strawberry pie, apple, pumpkin and so many others,\
 but there is no grape pie! I know. I'm just as upset about this unfortunate lack of development in the pie division.\
  Think about it. Grapes are used to make jelly, jam, juice and raisins. What makes them undesirable for pie? Would\
   they dry into raisins? Couldn't you just stick some jelly in a piecrust and bake it? It just doesn't make any sense.\
    Another thing that bothers me is organ grinders. You know, the foreign guys with the bellhop hats and the little\
     music thingy and the cute little monkey with the bellhop hat who collects the money? Okay. They're basically begging\
      on the street. How did they ever afford an organ-thingy? Wouldn't it make more sense to get a kazoo, if you're broke?\
       And if they're so poor, what possessed them to buy a monkey?");
        let res: String = convert_long_statements_to_short(
            terminal_size,
            marker_range_relative,
            &mut marker_size,
            test_word_ref,
            &mut offset,
            indent,
            padding_till_literal_start,
        );
        assert_eq!(
            res,
            "  ... Speak...ggg of food, what's up with pie? There's strawberry pie ..."
        );
    }
}
