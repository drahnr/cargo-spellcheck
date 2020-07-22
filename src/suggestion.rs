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

use crate::documentation::{CheckableChunk, ContentOrigin};

use std::cmp;
use std::convert::TryFrom;

use enumflags2::BitFlags;

use crate::{LineColumn, Range, Span};
use std::path::{Path, PathBuf};

/// Bitflag of available checkers by compilation / configuration.
#[derive(Debug, Clone, Copy, BitFlags, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum Detector {
    Hunspell = 0b0001,
    LanguageTool = 0b0010,
    #[cfg(test)]
    Dummy = 0b1000,
}

pub fn get_terminal_size() -> usize {
    use super::*;
    const DEFAULT_TERMINAL_SIZE: usize = 80;
    match crossterm::terminal::size() {
        Ok((terminal_size, _)) => terminal_size as usize,
        Err(_) => {
            warn!(
                "Unable to get terminal size. Using default: {}",
                DEFAULT_TERMINAL_SIZE
            );
            DEFAULT_TERMINAL_SIZE
        }
    }
}

pub fn get_current_statement<'a>(arr: &'a Vec<&'_ str>, range: Range) -> (&'a str, usize) {
    let mut stripped_line: &str = "";
    let mut initial_sentence: usize = 0;
    let mut line_pos: usize = 0;
    for (pos, sentence) in arr.iter().enumerate() {
        initial_sentence += sentence.chars().count();
        line_pos = pos;
        stripped_line = sentence;
        if range.end < initial_sentence {
            break;
        }
    }
    (stripped_line, line_pos)
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
// @TODO: This needs to be removed this. This is just an example to show all cases that I came up.
/// Speaking of food, what's up with pie? There's strawberry pie, apple, pumpkin and so many others, but there is no grape pie! I know. I'm just as upset about this unfortunate lack of development in the pie division. Think about it. Grapes are used to make jelly, jam, juice and raisins. What makes them undesirable for pie? Would they dry into raisins? Couldn't you just stick some jelly in a piecrust and bake it? It just doesn't make any sense. Another thing that bothers me is organ grinders. You know, the foreign guys with the bellhop hats and the little music thingy and the cute little monkey with the bellhop hat who collects the money? Okay. They're basically begging on the street. How did they ever afford an organ-thingy? Wouldn't it make more sense to get a kazoo, if you're broke? And if they're so poor, what possessed them to buuuuuuuuuuuuuyyyyyyyyyyyyyyyyyyyyy a monkey?
/// Speaking of food, what's up with pie? There's strawberry pie, apple, pumpkin and so many others, but there is no grape pie! I know. I'm just as upset about this unfortunate lack of development in the pie division. Think about it. Grapes are used to make jelly, jam, juice and raisins. What makes them undesirable for pie? Would they dry into raisins? Couldn't you just stick some jelly in a piecrust and bake it? It just doesn't make any sense. Another thing that bothers me is organ grinders. You know, the foreign guys with the bellhop hats and the little music thingy and the cute little monkey with the bellhop hat who collects the money? Okay. They're basically begging on the street. How did they ever afford an organ-thingy? Wouldn't it make more sense to get a kazoo, if you're broke? And if they're so poor, what possessed them to buyi a monkey?
/// Speakiiiiiinnnnnnnnnnnnnnngggggggg of food, what's up with pie? There's strawberry pie, apple, pumpkin and so many others, but there is no grape pie! I know. I'm just as upset about this unfortunate lack of development in the pie division. Think about it. Grapes are used to make jelly, jam, juice and raisins. What makes them undesirable for pie? Would they dry into raisins? Couldn't you just stick some jelly in a piecrust and bake it? It just doesn't make any sense. Another thing that bothers me is organ grinders. You know, the foreign guys with the bellhop hats and the little music thingy and the cute little monkey with the bellhop hat who collects the money? Okay. They're basically begging on the street. How did they ever afford an organ-thingy? Wouldn't it make more sense to get a kazoo, if you're broke? And if they're so poor, what possessed them to buy a monkey?
/// Speakingi of food, what's up with pie? There's strawberry pie, apple, pumpkin and so many others, but there is no grape pie! I know. I'm just as upset about this unfortunate lack of development in the pie division. Think about it. Grapes are used to make jelly, jam, juice and raisins. What makes them undesirable for pie? Would they dry into raisins? Couldn't you just stick some jelly in a piecrust and bake it? It just doesn't make any sense. Another thing that bothers me is organ grinders. You know, the foreign guys with the bellhop hats and the little music thingy and the cute little monkey with the bellhop hat who collects the money? Okay. They're basically begging on the street. How did they ever afford an organ-thingy? Wouldn't it make more sense to get a kazoo, if you're broke? And if they're so poor, what possessed them to buy a monkey?
/// pneumonoultramicroscopicsilicovolcanoconiose of food, what's up with pie? There's strawberry pie, apple, pumpkin and so many others, but there is no grape pie! I know. I'm just as upset about this unfortunate lack of development in the pie division. Think about it. Grapes are used to make jelly, jam, juice and raisins. What makes them undesirable for pie? Would they dry into raisins? Couldn't you just stick some jelly in a piecrust and bake it? It just doesn't make any sense. Another thing that bothers me is organ grinders. You know, the foreign guys with the bellhop hats and the little music thingy and the cute little monkey with the bellhop hat who collects the money? Okay. They're basically begging on the street. How did they ever afford an organ-thingy? Wouldn't it make more sense to get a kazoo, if you're broke? And if they're so poor, what possessed them to buy a monkey?
/// Reasn of food, what's up with pie? There's strawberry pie, apple, pumpkin and so many others, but there is no grape pie! I know. I'm just as upset about this unfortunate lack of development in the pie division. Think about it. Grapes are used to make jelly, jam, juice and raisins. What makes them undesirable for pie? Would they dry into raisins? Couldn't you just stick some jelly in a piecrust and bake it? It just doesn't make any sense. Another thing that bothers me is organ grinders. You know, the foreign guys with the bellhop hats and the little music thingy and the cute little monkey with the bellhop hat who collects the money? Okay. They're basically begging on the street. How did they ever afford an organ-thingy? Wouldn't it make more sense to get a kazoo, if you're broke? And if they're so poor, what possessed them to buy a monkey?
/// Speaking of food, what's up with pie? There's strawberry pie, apple, pumpkin and so many others, but therie is no grape pie! I know. I'm just as upset about this unfortunate lack of development in the pie division.
/// Speaking of food, what's up with pie? There's strawberry pie, apple, pumpkin and so many others, but therieeeeeeeeeeeeeeee is no grape pie! I know. I'm just as upset about this unfortunate lack of development in the pie division.
/// Speaking of food, what's up with pie? There's strawberry pie, apple, pumpkin and so many otherrrrrrrrrrrrrs, but therieeeeeeeeeeeeeeee is no gruuuuuuuuuuuuuuuuuuuuuuuape pie! I know. I'm just as upset about this unfortunate lack of development in the pie division.
impl fmt::Display for Detector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::LanguageTool => "LanguageTool",
            Self::Hunspell => "Hunspell",
            #[cfg(test)]
            Self::Dummy => "Dummy",
        })
    }
}

// For long lines, literal will be trimmed to display in one terminal line.
// Misspelled words that are too long shall also be ellipsized.
pub fn convert_long_statements_to_short(
    terminal_size: usize,
    indent: usize,
    stripped_line: &str,
    offset: &mut usize,
    range_word: Range,
    padding_till_excerpt_start: usize,
    marker_size: &mut usize,
) -> String {
    use super::*;
    // The paddings give some space for the ` {} ...` and extra indentation and formatting:
    //
    //|---------------------------------------------------------------------------------------| terminal_size
    //|-------| padding_till_excerpt_start = indent (3+line_number_digit_count) + 2 white spaces = 7usize, for this case.
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
    // |----------------------------------too long line---------------------------------|
    // |----------------------|---------misspelled_word---------|-----------------------|
    // |-----left_context-----|range_start_word|-range_end_word-|-----right_context-----|
    //
    // Obs: paddings are not being considered in the illustration, but info is above.

    // the line being analysed can affect how the indentation is done.
    let mut range_start_word = Range {
        start: range_word.start,
        end: range_word.start,
    };
    let mut range_end_word = Range {
        start: range_word.end,
        end: range_word.end,
    };
    let mut misspelled_word: String = stripped_line
        .chars()
        .skip(range_word.start)
        .take(range_word.len())
        .collect();

    // Misspelled words that are too long will be formatted for fitting.
    if range_word.len() > TOO_LONG_WORD {
        range_start_word = Range {
            start: range_word.start,
            end: range_start_word.start + DISPLAYED_LONG_WORD,
        };
        range_end_word = Range {
            start: range_word
                .end //non inclusive
                .saturating_sub(DISPLAYED_LONG_WORD),
            end: range_word.end,
        };

        //  too long word will be shorter as it follows:
        //    4 chars |----|  ... |---| 3 chars
        //                ther...eee, for therieeeeeeeeeeeeeeee
        //
        misspelled_word = format!(
            "{}...{}",
            stripped_line
                .chars()
                .skip(range_start_word.start)
                .take(range_start_word.len())
                .collect::<String>(),
            stripped_line
                .chars()
                .skip(range_start_word.end.saturating_sub(3))
                .take(3)
                .collect::<String>()
        );
        *marker_size = misspelled_word.chars().count();
    }
    let available_space = (terminal_size.saturating_sub(
        misspelled_word.chars().count() + padding_till_excerpt_start + PADDING_AROUND_LONG_LINES,
    )) / 2;
    let mut left_context = Range {
        start: 0,
        end: range_word.start,
    };
    let mut right_context = Range {
        start: range_word.end,
        end: stripped_line.chars().count(),
    };
    let left_remaining_space: i32 = available_space as i32 - left_context.len() as i32;
    let right_remaining_space: i32 = available_space as i32 - right_context.len() as i32;

    match (left_remaining_space > 0, right_remaining_space > 0) {
        (true, false) => {
            let right_available_space_recalculated =
                2 * available_space as i32 - left_context.len() as i32;
            right_context.end = cmp::min(
                (range_word.end as i32 + right_available_space_recalculated as i32) as usize,
                stripped_line.chars().count(),
            )
        }
        (false, true) => {
            let left_available_space_recalculated =
                2 * available_space as i32 - right_context.len() as i32;
            left_context.start = cmp::max(
                (range_word.start as i32 - left_available_space_recalculated as i32) as usize,
                0usize,
            );
        }
        (false, false) => {
            left_context.start = range_word.start - available_space;
            right_context.end = range_word.end + available_space;
        }
        _ => (),
    };
    *offset = left_context.len() + PADDING_OFFSET;
    format!(
        "  ... {}{}{} ...",
        stripped_line
            .chars()
            .skip(left_context.start)
            .take(left_context.len())
            .collect::<String>(),
        misspelled_word,
        stripped_line
            .chars()
            .skip(right_context.start)
            .take(right_context.len())
            .collect::<String>()
    )
}
// Formatting itself added white spaces and punctuation to do the fitting to be considered:
//
//     |------ info ----| => PADDING_AROUND_LONG_LINES = 10 usize

/// A suggestion for certain offending span.
#[derive(Clone, Hash, PartialEq, Eq)]
pub struct Suggestion<'s> {
    /// Which checker suggested the change.
    pub detector: Detector,
    /// Reference to the file location the `span` and `literal` relate to.
    pub origin: ContentOrigin,
    /// @todo must become a `CheckableChunk` and properly integrated
    pub chunk: &'s CheckableChunk,
    /// The span (absolute!) within the file or chunk (depens on `origin`).
    pub span: Span,
    /// Range relative to the chunk the current suggestion is located.
    pub range: Range,
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

        let x = self.span.start.line;
        let (path, line) = match self.origin {
            ContentOrigin::RustDocTest(ref path, ref span) => {
                (path.display().to_string(), x + span.start.line)
            }
            ref origin => (origin.as_path().display().to_string(), x),
        };
        writeln!(formatter, " {path}:{line}", path = path, line = line)?;
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
            self.chunk
                .len_in_chars()
                .saturating_sub(self.span.start.column)
        };

        let literal_span: Span = self.span.clone();
        let marker_range_relative: Range = self.range.clone();

        // if the offset starts from 0, we still want to continue if the length
        // of the marker is at least length 1.
        let mut offset = marker_range_relative.start;
        let mut v = self
            .chunk
            .as_str()
            .lines()
            .enumerate()
            .map(|(lineno, content)| (lineno + 1, content))
            .skip_while(|(lineno, _)| &self.span.start.line < lineno)
            .take_while(|(lineno, _)| &self.span.end.line >= lineno)
            .map(|(_, content)| content)
            .collect::<Vec<&'_ str>>();

        let (stripped_line, pos) = get_current_statement(&v.as_ref(), self.range.clone());
        let chars_till_start_statement = v[0..pos].iter().fold(0, |sum, x| sum + x.chars().count());
        let range_word: Range = Range {
            start: self.range.start.saturating_sub(chars_till_start_statement),
            end: self.range.end.saturating_sub(chars_till_start_statement),
        };

        let terminal_size = get_terminal_size();

        // this values is dynamically calculated for each line where the doc is.
        let padding_till_excerpt_start = indent + 2;

        // Check whether the statement is too long the terminal size for fitting purposes.
        if stripped_line.char_indices().count() + padding_till_excerpt_start > terminal_size {
            let formatted_literal = convert_long_statements_to_short(
                terminal_size,
                indent,
                stripped_line,
                &mut offset,
                range_word,
                padding_till_excerpt_start,
                &mut marker_size,
            );
            writeln!(formatter, "{}", formatted_literal)?;

        // literal is smaller than terminal size and it can be fully displayed.
        } else {
            offset = offset.saturating_sub(chars_till_start_statement);
            writeln!(formatter, "  {}", stripped_line)?;
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
        // @todo
        // log::trace!(
        //     "marker_size={} [{}|{}|{}] literal {{ {:?} .. {:?} }} >> {:?} <<",
        //     marker_size,
        //     self.chunk.pre(),
        //     self.chunk.len(),
        //     self.chunk.post(),
        //     self.span.start,
        //     self.span.end,
        //     self,
        // );
        } else {
            // @todo
            // log::warn!(
            //     "marker_size={} [{}|{}|{}] literal {{ {:?} .. {:?} }} >> {:?} <<",
            //     marker_size,
            //     self.chunk.pre(),
            //     self.chunk.len(),
            //     self.chunk.post(),
            //     self.span.start,
            //     self.span.end,
            //     self,
            // );
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
        match crate::documentation::ChunkDisplay::try_from((self.chunk, self.span)) {
            Ok(printable) => write!(formatter, "({}, {:?})", &printable, printable.1),
            Err(e) => {
                writeln!(formatter, "> span={:?}", self.span)?;
                writeln!(
                    formatter,
                    "> Failed to create chunk display from chunk={:?} with {}",
                    self.chunk, e
                )
            }
        }
    }
}

/// A set of suggestions across multiple files, clustered per file
#[derive(Debug, Clone)]
pub struct SuggestionSet<'s> {
    per_file: indexmap::IndexMap<ContentOrigin, Vec<Suggestion<'s>>>,
}

impl<'s> SuggestionSet<'s> {
    pub fn new() -> Self {
        Self {
            per_file: indexmap::IndexMap::with_capacity(64),
        }
    }

    pub fn iter<'a>(
        &'a self,
    ) -> impl DoubleEndedIterator<Item = (&'a ContentOrigin, &'a Vec<Suggestion<'s>>)> {
        self.per_file.iter()
    }

    pub fn add(&mut self, origin: ContentOrigin, suggestion: Suggestion<'s>) {
        self.per_file
            .entry(origin)
            .or_insert_with(|| Vec::with_capacity(1))
            .push(suggestion);
    }

    pub fn append(&mut self, origin: ContentOrigin, suggestions: &[Suggestion<'s>]) {
        self.per_file
            .entry(origin)
            .or_insert_with(|| Vec::with_capacity(32))
            .extend_from_slice(suggestions);
    }

    pub fn extend<I>(&mut self, origin: ContentOrigin, suggestions: I)
    where
        I: IntoIterator<Item = Suggestion<'s>>,
    {
        let v: &mut Vec<Suggestion<'s>> = self
            .per_file
            .entry(origin)
            .or_insert_with(|| Vec::with_capacity(32));
        v.extend(suggestions.into_iter());
    }

    /// Obtain an accessor `Entry` for the given `origin`
    pub fn entry(
        &mut self,
        origin: ContentOrigin,
    ) -> indexmap::map::Entry<ContentOrigin, Vec<Suggestion<'s>>> {
        self.per_file.entry(origin)
    }

    /// Iterate over all files by reference
    pub fn files<'i, 'a>(&'a mut self) -> impl DoubleEndedIterator<Item = &'i ContentOrigin>
    where
        's: 'i,
        'a: 'i,
    {
        self.per_file.keys()
    }

    /// Iterate over all references given an origin
    ///
    /// panics if there is no such origin
    pub fn suggestions<'a>(
        &'a self,
        origin: &ContentOrigin,
    ) -> impl DoubleEndedIterator<Item = &'a Suggestion<'s>>
    where
        'a: 's,
    {
        if let Some(ref suggestions) = self.per_file.get(origin) {
            suggestions.iter()
        } else {
            panic!("origin must exist")
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
        I: IntoIterator<Item = (ContentOrigin, Vec<Suggestion<'s>>)>,
    {
        other.into_iter().for_each(|(origin, suggestions)| {
            self.entry(origin)
                .or_insert_with(|| Vec::with_capacity(suggestions.len()))
                .extend_from_slice(suggestions.as_slice())
        })
    }

    /// Obtain the number of items in the set
    #[inline]
    pub fn len(&self) -> usize {
        self.per_file.len()
    }

    /// Count the number of suggestions accross all files in total
    pub fn total_count(&self) -> usize {
        self.per_file.iter().map(|(_origin, vec)| vec.len()).sum()
    }
}

impl<'s> IntoIterator for SuggestionSet<'s> {
    type Item = (ContentOrigin, Vec<Suggestion<'s>>);
    type IntoIter = indexmap::map::IntoIter<ContentOrigin, Vec<Suggestion<'s>>>;
    fn into_iter(self) -> Self::IntoIter {
        self.per_file.into_iter()
    }
}

impl<'s> IntoIterator for &'s SuggestionSet<'s> {
    type Item = (&'s ContentOrigin, &'s Vec<Suggestion<'s>>);
    type IntoIter = indexmap::map::Iter<'s, ContentOrigin, Vec<Suggestion<'s>>>;
    fn into_iter(self) -> Self::IntoIter {
        self.per_file.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LineColumn;
    use console;
    use std::fmt;
    fn assert_display_eq<D: fmt::Display, S: AsRef<str>>(display: D, s: S) {
        let expected = s.as_ref();
        let expected = console::strip_ansi_codes(expected);

        // uses the display impl
        let reality = display.to_string();
        let reality = console::strip_ansi_codes(reality.as_str());
        assert_eq!(reality, expected);
    }

    #[test]
    fn fmt() {
        const CONTENT: &'static str = "Is it dyrck again?";
        let chunk = CheckableChunk::from_str(
            CONTENT,
            indexmap::indexmap! { 0..18 => Span {
                    start: LineColumn {
                        line: 1,
                        column: 0,
                    },
                    end: LineColumn {
                        line: 1,
                        column: 17,
                    }
                }
            },
        );

        let suggestion = Suggestion {
            detector: Detector::Dummy,
            origin: ContentOrigin::TestEntity,
            chunk: &chunk,
            range: 6..11,
            span: Span {
                start: LineColumn { line: 1, column: 6 },
                end: LineColumn {
                    line: 1,
                    column: 10,
                },
            },
            replacements: vec!["replacement_0", "replacement_1", "replacement_2"]
                .into_iter()
                .map(std::borrow::ToOwned::to_owned)
                .collect(),
            description: Some("Possible spelling mistake found.".to_owned()),
        };

        const EXPECTED: &'static str = r#"error: spellcheck(Dummy)
  --> /tmp/test/entity:1
   |
 1 | Is it dyrck again?
   |       ^^^^^
   | - replacement_0, replacement_1, or replacement_2
   |
   |   Possible spelling mistake found.
   |
"#;

        assert_display_eq(suggestion, EXPECTED);
    }

    #[test]
    fn multiline_is_dbg_printable() {
        let _ = env_logger::builder().is_test(true).try_init();

        use crate::documentation::CheckableChunk;
        let chunk = CheckableChunk::from_str(
            r#"0
2345
7@n"#,
            indexmap::indexmap! { 0..10 => Span {
                start : LineColumn {
                    line: 7usize,
                    column: 8usize,
                },
                end : LineColumn {
                    line: 9usize,
                    column: 4usize,
                }
            } },
        );

        let suggestion = Suggestion {
            detector: Detector::Dummy,
            origin: ContentOrigin::TestEntity,
            chunk: &chunk,
            span: Span {
                start: LineColumn {
                    line: 8usize,
                    column: 0,
                },
                end: LineColumn {
                    line: 8usize,
                    column: 3,
                },
            },
            range: 2..6,
            replacements: vec!["whocares".to_owned()],
            description: None,
        };

        let suggestion = dbg!(suggestion);

        log::info!("fmt debug=\n{:?}\n<", suggestion);
        log::info!("fmt display=\n{}\n<", suggestion);
    }
}
