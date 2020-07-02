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

use crate::Span;
use crate::documentation::{CheckableChunk, ContentOrigin};
use std::path::{Path, PathBuf};
use std::convert::TryFrom;

use crossterm::terminal::size;
use enumflags2::BitFlags;


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

/// A suggestion for certain offending span.
#[derive(Clone, Hash, PartialEq, Eq)]
pub struct Suggestion<'s> {
    /// Which checker suggested the change.
    pub detector: Detector,
    /// Reference to the file location the `span` and `literal` relate to.
    pub origin: ContentOrigin,
    /// @todo must become a `CheckableChunk` and properly integrated
    pub chunk: &'s CheckableChunk,
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
        let _help = Style::new().yellow().bold();

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
            ContentOrigin::RustSourceFile(ref path) => (path.display().to_string(), x),
            ContentOrigin::RustDocTest(ref path, ref span) => {
                (path.display().to_string(), x + span.start.line)
            }
            ContentOrigin::CommonMarkFile(ref path) => (path.display().to_string(), x),
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
            self.literal.len().saturating_sub(self.span.start.column)
        };

        use crate::literalset::Range;

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

        // For long lines, we will trim the literal displayed to fit in the terminal
        // The misspelled word shall always be shown with as much info as possible
        // Misspelled words that are too long shall also be ellipsized

        // The paddings give some space for the ` {} ...` and extra indentation and formatting:
        // ```
        // 65 |  ...  here, on the second most pointless site ever! Well, wander ont  ...
        //    |                                                                  ^^^
        //    |                                                                                                                                                                                                ^^^
        //    | - not, Ont, int, on, onto, cont, font, or one of 9 others
        //    |
        //    |   Possible spelling mistake found.
        //    |
        // ```
        const PADDING_OFFSET: usize = 6;
        const PADDING_OFFSET_START: usize = 4;
        const PADDING_END: usize = 15;
        const TOO_LONG_WORD: usize = 20;
        const DISPLAYED_LONG_WORD: usize = 4usize;

        let terminal_size: usize = get_terminal_size();
        // We will be using ranges to help doing the fitting:
        //
        // |----------------------------------literal_word----------------------------------|
        // |----------------------|---------misspelled_word---------|-----------------------|
        // |-----left_context-----|---start_word----|----end_word---|-----right_context-----|
        //
        let mut range_left_context = Range {
            start: 0usize,
            end: marker_range_relative.start,
        };
        let mut range_right_context = Range {
            start: marker_range_relative.end,
            end: self.literal.as_str().chars().count(),
        };
        let mut range_start_word = Range {
            start: marker_range_relative.start,
            end: marker_range_relative.start,
        };
        let mut range_end_word = Range {
            start: marker_range_relative.end,
            end: marker_range_relative.end,
        };
        if self.literal.as_str().chars().count() > terminal_size {
            let mut misspelled_word = format!(
                "{}",
                self.literal.sub(Range {
                    start: marker_range_relative.start - 1,
                    end: marker_range_relative.end
                })
            );
            if marker_size > TOO_LONG_WORD {
                range_start_word.start = marker_range_relative.start - 1;
                range_start_word.end = range_start_word.start + DISPLAYED_LONG_WORD;

                range_end_word.start = marker_range_relative
                    .end
                    .saturating_sub(DISPLAYED_LONG_WORD);
                range_end_word.end = marker_range_relative.end;

                misspelled_word = format!(
                    "{}...{}",
                    self.literal.sub(range_start_word),
                    self.literal.sub(range_end_word)
                );
                marker_size = misspelled_word.chars().count();
            };
            // right context has enough info to fill the terminal
            //  |-----misspelled_word-----|--------right_context---------|
            //
            // Attempt to fit the misspelled word in the beginning followed by info.
            if range_right_context.len() >= terminal_size {
                // Left range will not be used in this case
                range_left_context.start = 0usize;
                range_left_context.end = 0usize;

                range_right_context.start = marker_range_relative.end;
                range_right_context.end = marker_range_relative.end
                    + (terminal_size
                        .saturating_sub(misspelled_word.chars().count() + PADDING_END + 1));
                offset = offset.saturating_sub(range_left_context.start) + PADDING_OFFSET_START;
            }
            // left context has enough info to fill the terminal
            // |---------left_context---------|-----misspelled_word-----|
            //
            // Attempt to fit the misspelled word with left context info
            else if range_left_context.len() > terminal_size {
                range_left_context.start = marker_range_relative.start.saturating_sub(
                    terminal_size.saturating_sub(misspelled_word.chars().count() + PADDING_END),
                );
                range_left_context.end = marker_range_relative.start - 1;
                // Right range will not be used
                range_right_context.start = 0usize;
                range_right_context.end = 0usize;

                offset = offset.saturating_sub(range_left_context.start + 1) + PADDING_OFFSET;
            }
            // information will be shown in both sides of the `misspelled_word`
            // |--left_context--|----misspelled_word---|--right_context--|
            //
            // Attempt to fit the misspelled word in the middle with info in th left and right of it
            else {
                let context = (terminal_size.saturating_sub(misspelled_word.chars().count())) / 2;
                range_left_context.end = marker_range_relative.start;
                range_left_context.start = range_left_context.end.saturating_sub(context);

                range_right_context.start = marker_range_relative.end;
                range_right_context.end = marker_range_relative
                    .end
                    .saturating_add(context)
                    .saturating_sub(PADDING_END);
                offset = offset.saturating_sub(range_left_context.start + 1) + PADDING_OFFSET;
            }
            writeln!(
                formatter,
                "  ... {}{}{} ... ",
                self.literal.sub(range_left_context),
                misspelled_word,
                self.literal.sub(range_right_context)
            )?;
        // literal is smaller than terminal size and can be fully displayed
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
