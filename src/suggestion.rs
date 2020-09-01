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

use crate::{Range, Span};

/// Bitflag of available checkers by compilation / configuration.
#[derive(Debug, Clone, Copy, BitFlags, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum Detector {
    /// Hunspell lib based detector.
    Hunspell = 0b0001,
    /// Language tool server based detection.
    LanguageTool = 0b0010,
    /// Reflow according to a given max column.
    Reflow = 0b0100,
    /// Detection of nothing, a test helper.
    #[cfg(test)]
    Dummy = 0b1000,
}

/// Terminal size in characters.
///
/// Returns `80usize` for tests and in case the terminal size
/// can not be retrieved.
pub fn get_terminal_size() -> usize {
    const DEFAULT_TERMINAL_SIZE: usize = 80;
    #[cfg(not(test))]
    match crossterm::terminal::size() {
        Ok((terminal_size, _)) => terminal_size as usize,
        Err(_) => {
            log::warn!(
                "Unable to get terminal size. Using default: {}",
                DEFAULT_TERMINAL_SIZE
            );
            DEFAULT_TERMINAL_SIZE
        }
    }
    #[cfg(test)]
    DEFAULT_TERMINAL_SIZE
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
            Self::Reflow => "Reflow",
            #[cfg(test)]
            Self::Dummy => "Dummy",
        })
    }
}

/// For long lines, literal will be trimmed to display in one terminal line.
/// Misspelled words that are too long shall also be ellipsized.
pub fn condition_display_content(
    terminal_size: usize,
    _indent: usize,
    stripped_line: &str,
    mistake_range: Range,
    terminal_print_offset_left: usize,
    marker_size: usize,
) -> (String, usize, usize) {
    // if we can fit the full line in there, avoid all the work as much as possible
    if stripped_line.chars().count() + terminal_print_offset_left <= terminal_size {
        return (stripped_line.to_owned(), mistake_range.start, marker_size);
    }

    // The paddings give some space for the ` {} ...` and extra indentation and formatting:
    //
    //|---------------------------------------------------------------------------------------| terminal_size
    //|-------| padding_till_excerpt_start = indent (3+line_number_digit_count) + 2 white spaces = 7usize, for this case.
    //
    //   --> /home/tmhdev/Documents/cargo-spellcheck/src/suggestion.rs:62
    //    |
    // 62 |  Reasn of food, what's up with pie? There's strawberry pie, apple, pumpkin..
    //    |  ^^^^^
    //    | - there, Cherie, thither, or tither
    //    |
    //    |   Possible spelling mistake found.
    //
    const MAX_MISTAKE_LEN: usize = 20;

    const HEAD_DISPLAY_LEN: usize = 4;
    const TAIL_DISPLAY_LEN: usize = 4;

    const CENTER_DOTS: &'static str = "...";
    const LEFT_DOTS: &'static str = "..";
    const RIGHT_DOTS: &'static str = "..";
    const NO_DOTS: &'static str = "";

    // guarantees that `marker_size` is always less than the max length.
    assert!(HEAD_DISPLAY_LEN + CENTER_DOTS.len() + TAIL_DISPLAY_LEN <= MAX_MISTAKE_LEN);

    // worst case conservative estimate, should be calculated based on `indent`
    const TOTAL_CONTEXT_CHAR_COUNT: usize = 6;

    // We will be using ranges to help doing the fitting:
    //
    // |-----------------------------------excerpt--------------------------------------|
    // |----------------------|---------misspelled_word---------|-----------------------|
    // |-----left_context-----|head_sub_range|-tail_sub_range-|-----right_context-----|
    //
    // Obs: paddings are not being considered in the illustration, but info is above.

    // Misspelled words that are too long will be shortened by ellipsizing parts of it.
    let (marker_size, shortened) = if mistake_range.len() > MAX_MISTAKE_LEN {
        let head_sub_range = Range {
            start: mistake_range.start,
            end: mistake_range.start + HEAD_DISPLAY_LEN,
        };
        let tail_sub_range = Range {
            start: mistake_range
                .end //non inclusive
                .saturating_sub(TAIL_DISPLAY_LEN),
            end: mistake_range.end,
        };

        //  too long word will be shorter as it follows:
        //            |-------------------| > MAX_MISTAKE_LEN
        //            therieeeeeeeeeeeeeeee
        //   4 chars  ^^^^   ...       ^^^^  4 chars
        //
        //  result:      ther...eeee

        let head_sub = stripped_line
            .chars()
            .skip(head_sub_range.start)
            .take(HEAD_DISPLAY_LEN)
            .collect::<String>();
        let tail_sub = stripped_line
            .chars()
            .skip(tail_sub_range.start)
            .take(TAIL_DISPLAY_LEN)
            .collect::<String>();

        let shortened = format!("{}...{}", head_sub, tail_sub);
        let marker_size = head_sub_range.len() + CENTER_DOTS.len() + tail_sub_range.len();

        (marker_size, shortened)
    } else {
        let full: String = stripped_line
            .chars()
            .skip(mistake_range.start)
            .take(mistake_range.len())
            .collect();
        (marker_size, full)
    };

    let stripped_line_len = stripped_line.chars().count();

    // full, uncut context coverage
    let left_context = Range {
        start: 0,
        end: mistake_range.start,
    };
    let right_context = Range {
        start: mistake_range.end,
        end: stripped_line_len,
    };

    let avail_space = terminal_size
        .saturating_sub(terminal_print_offset_left + marker_size + TOTAL_CONTEXT_CHAR_COUNT);

    // left and right we would like to partition the remaining space equally
    let avail_space_half = avail_space / 2usize;

    // TODO introduce a threshold, so the shortened version is not longer than than the original
    let (left_context, right_context, left_dots, right_dots) = match (
        avail_space_half > left_context.len(),
        avail_space_half > right_context.len(),
    ) {
        (true, false) => {
            // left context does not use all the capacity avail
            // allow the right context to consume the excess.
            let right_avail_space = avail_space - left_context.len();
            let rdots = if mistake_range.end + right_avail_space < stripped_line_len {
                NO_DOTS
            } else {
                RIGHT_DOTS
            };
            (
                left_context,
                Range {
                    start: right_context.end,
                    end: cmp::min(mistake_range.end + right_avail_space, stripped_line_len),
                },
                NO_DOTS,
                rdots,
            )
        }
        (false, true) => {
            // right context does not use all the capacity avail
            // allow the left context to consume the excess.
            let left_avail_space = avail_space - right_context.len();
            let ldots = if left_avail_space > left_context.end {
                NO_DOTS
            } else {
                LEFT_DOTS
            };
            (
                Range {
                    start: left_context.end.saturating_sub(left_avail_space),
                    end: left_context.end,
                },
                right_context,
                ldots,
                NO_DOTS,
            )
        }
        (false, false) => {
            // both sides have excess chars, so yield `avail_space_half` to both sides
            (
                Range {
                    start: left_context.end.saturating_sub(avail_space_half),
                    end: left_context.end,
                },
                Range {
                    start: right_context.start,
                    end: right_context.start + avail_space_half,
                },
                LEFT_DOTS,
                RIGHT_DOTS,
            )
        }
        _ => {
            // both sides are less than the allowed context, no need to modify
            (left_context, right_context, NO_DOTS, NO_DOTS)
        }
    };

    assert!(left_context.end == mistake_range.start);
    assert!(right_context.end <= stripped_line_len);
    assert!(left_context.len() + mistake_range.len() + right_context.len() <= stripped_line_len);

    let offset = left_context.len();
    let conditioned_line = format!(
        "{}{}{}{}{}",
        left_dots,
        stripped_line
            .chars()
            .skip(left_context.start + left_dots.len())
            .take(left_context.len() - left_dots.len())
            .collect::<String>(),
        shortened,
        stripped_line
            .chars()
            .skip(right_context.start)
            .take(right_context.len() - right_dots.len())
            .collect::<String>(),
        right_dots,
    );
    (conditioned_line, offset, marker_size)
}

/// A suggestion for certain offending span.
#[derive(Clone, Hash, PartialEq, Eq)]
pub struct Suggestion<'s> {
    /// Which checker suggested the change.
    pub detector: Detector,
    /// Reference to the file location the `span` and `literal` relate to.
    pub origin: ContentOrigin,
    /// The suggestion is relative to a specific chunk.
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

        // TODO this needs some more thought once multiline comments pop up
        let marker_size = self.span.one_line_len().unwrap_or_else(|| {
            self.chunk
                .len_in_chars()
                .saturating_sub(self.span.start.column)
        });

        // assumes the _mistake_ is within one line
        // if not we chop it down to the first line
        let mistake_lines = self.chunk.find_covered_lines(self.range.clone());
        let (line_range, start_of_line_offset) = mistake_lines
            .first()
            .map(|line_range| {
                (
                    line_range,
                    self.range.start.saturating_sub(line_range.start),
                )
            })
            .expect("Lines covered must exist");

        let intra_line_mistake_range = Range {
            start: start_of_line_offset,
            end: cmp::min(start_of_line_offset + self.range.len(), line_range.len()),
        };
        let relevant_line = self
            .chunk
            .as_str()
            .chars()
            .enumerate()
            .skip_while(|(idx, _)| line_range.start > *idx)
            .take(line_range.len())
            .map(|(_, c)| c)
            .collect::<String>();

        let terminal_size = get_terminal_size();

        // this values is dynamically calculated for each line where the doc is.
        // the line being analysed can affect how the indentation is done.
        let padding_till_excerpt_start = indent + 2;

        let (formatted, offset, marker_size) = condition_display_content(
            terminal_size,
            indent,
            relevant_line.as_str(),
            intra_line_mistake_range,
            padding_till_excerpt_start,
            marker_size,
        );

        writeln!(formatter, " {}", formatted.as_str())?;

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
                "marker_size={} span {{ {:?} .. {:?} }} >> {:?} <<",
                marker_size,
                self.span.start,
                self.span.end,
                self,
            );
        } else {
            log::warn!(
                "marker_size={} span {{ {:?} .. {:?} }} >> {:?} <<",
                marker_size,
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

        if !self.replacements.is_empty() {
            formatter.write_str("\n")?;
            context_marker
                .apply_to(format!("{:>width$}", "|\n", width = indent + 1))
                .fmt(formatter)?;
            context_marker
                .apply_to(format!("{:>width$}", "|", width = indent))
                .fmt(formatter)?;
        }

        if let Some(ref description) = self.description {
            writeln!(formatter, "   {}", description)?;
        }
        Ok(())
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
    /// Create a new and empty suggestion set.
    pub fn new() -> Self {
        Self {
            per_file: indexmap::IndexMap::with_capacity(64),
        }
    }

    /// Iterate over all suggestions tupled with the content origin of the file the
    /// suggestion relates to.
    pub fn iter<'a>(
        &'a self,
    ) -> impl DoubleEndedIterator<Item = (&'a ContentOrigin, &'a Vec<Suggestion<'s>>)> {
        self.per_file.iter()
    }

    /// Adds a new suggestion to the set.
    pub fn add(&mut self, origin: ContentOrigin, suggestion: Suggestion<'s>) {
        self.per_file
            .entry(origin)
            .or_insert_with(|| Vec::with_capacity(1))
            .push(suggestion);
    }

    /// Adds a slice of suggestions at once.
    pub fn append(&mut self, origin: ContentOrigin, suggestions: &[Suggestion<'s>]) {
        self.per_file
            .entry(origin)
            .or_insert_with(|| Vec::with_capacity(32))
            .extend_from_slice(suggestions);
    }

    /// Alternative form of [`Self::append`](Self::append).
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

    /// Count the number of suggestions across all files in total
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
    use crate::{LineColumn, CommentVariant};
    use console;
    use std::fmt;

    /// A test helper comparing the output against an expected output.
    ///
    /// Strips all colour codes from both the expected string as well as the
    /// display-able object.
    fn assert_display_eq<D: fmt::Display, S: AsRef<str>>(display: D, s: S) {
        let expected = s.as_ref();
        let expected = console::strip_ansi_codes(expected);

        // uses the display impl
        let reality = display.to_string();
        let reality = console::strip_ansi_codes(reality.as_str());
        assert_eq!(reality, expected);
    }

    #[test]
    fn fmt_0_single() {
        const CONTENT: &'static str = " Is it dyrck again?";
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
            CommentVariant::TripleSlash,
        );

        let suggestion = Suggestion {
            detector: Detector::Dummy,
            origin: ContentOrigin::TestEntityRust,
            chunk: &chunk,
            range: 7..12,
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
  --> /tmp/test/entity.rs:1
   |
 1 |  Is it dyrck again?
   |        ^^^^^
   | - replacement_0, replacement_1, or replacement_2
   |
   |   Possible spelling mistake found.
"#;
        assert_display_eq(suggestion, EXPECTED);
    }

    #[test]
    fn fmt_0_no_suggestion() {
        const CONTENT: &'static str = " Is it dyrck again?";
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
            CommentVariant::TripleSlash,
        );

        let suggestion = Suggestion {
            detector: Detector::Dummy,
            origin: ContentOrigin::TestEntityRust,
            chunk: &chunk,
            range: 7..12,
            span: Span {
                start: LineColumn { line: 1, column: 6 },
                end: LineColumn {
                    line: 1,
                    column: 10,
                },
            },
            replacements: vec![],
            description: Some("Possible spelling mistake found.".to_owned()),
        };

        const EXPECTED: &'static str = r#"error: spellcheck(Dummy)
  --> /tmp/test/entity.rs:1
   |
 1 |  Is it dyrck again?
   |        ^^^^^
   |   Possible spelling mistake found.
"#;
        assert_display_eq(suggestion, EXPECTED);
    }

    #[test]
    fn fmt_1_multi() {
        const CONTENT: &'static str = r#" Line mitake 1
 Anowher 2
 Last"#;

        let chunk = CheckableChunk::from_str(
            CONTENT,
            indexmap::indexmap! {
                0..13 => Span {
                    start: LineColumn {
                        line: 1,
                        column: 4,
                    },
                    end: LineColumn {
                        line: 1,
                        column: 16,
                    }
                },
                14..24 => Span {
                    start: LineColumn {
                        line: 2,
                        column: 4,
                    },
                    end: LineColumn {
                        line: 2,
                        column: 12,
                    }
                },
                25..29 => Span {
                    start: LineColumn {
                        line: 3,
                        column: 4,
                    },
                    end: LineColumn {
                        line: 3,
                        column: 7,
                    }
                }
            },
            CommentVariant::TripleSlash,
        );

        let suggestion = Suggestion {
            detector: Detector::Dummy,
            origin: ContentOrigin::TestEntityRust,
            chunk: &chunk,
            range: 6..12,
            span: Span {
                start: LineColumn {
                    line: 1,
                    column: 10,
                },
                end: LineColumn {
                    line: 1,
                    column: 15,
                },
            },
            replacements: vec!["replacement_0", "replacement_1", "replacement_2"]
                .into_iter()
                .map(std::borrow::ToOwned::to_owned)
                .collect(),
            description: Some("Possible spelling mistake found.".to_owned()),
        };

        const EXPECTED: &'static str = r#"error: spellcheck(Dummy)
  --> /tmp/test/entity.rs:1
   |
 1 |  Line mitake 1
   |       ^^^^^^
   | - replacement_0, replacement_1, or replacement_2
   |
   |   Possible spelling mistake found.
"#;

        assert_display_eq(suggestion, EXPECTED);
    }

    #[test]
    fn fmt_2_multi_80_plus() {
        const CONTENT: &'static str = r#" Line mitake 1
 Suuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuper duuuuuuuuuuuuuuuuuuuuuuuuper too long
 "#;

        let chunk = CheckableChunk::from_str(
            CONTENT,
            indexmap::indexmap! {
                0..13 => Span {
                    start: LineColumn {
                        line: 1,
                        column: 4,
                    },
                    end: LineColumn {
                        line: 1,
                        column: 16,
                    }
                },
                14..101 => Span {
                    start: LineColumn {
                        line: 2,
                        column: 4,
                    },
                    end: LineColumn {
                        line: 2,
                        column: 90,
                    }
                }
            },
            CommentVariant::TripleSlash,
        );

        let suggestion = Suggestion {
            detector: Detector::Dummy,
            origin: ContentOrigin::TestEntityRust,
            chunk: &chunk,
            range: 66..94,
            span: Span {
                start: LineColumn { line: 2, column: 5 },
                end: LineColumn {
                    line: 2,
                    column: 92,
                },
            },
            replacements: vec!["replacement_0", "replacement_1", "replacement_2"]
                .into_iter()
                .map(std::borrow::ToOwned::to_owned)
                .collect(),
            description: Some("Possible spelling mistake found.".to_owned()),
        };

        const EXPECTED: &'static str = r#"error: spellcheck(Dummy)
  --> /tmp/test/entity.rs:2
   |
 2 | ..uuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuper duuu...uper too long
   |                                                 ^^^^^^^^^^^
   | - replacement_0, replacement_1, or replacement_2
   |
   |   Possible spelling mistake found.
"#;

        assert_display_eq(suggestion, EXPECTED);
    }

    #[test]
    fn multiline_is_dbg_printable() {
        let _ = env_logger::builder()
            .is_test(true)
            .filter(None, log::LevelFilter::Trace)
            .try_init();

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
            CommentVariant::TripleSlash,
        );

        let suggestion = Suggestion {
            detector: Detector::Dummy,
            origin: ContentOrigin::TestEntityRust,
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
