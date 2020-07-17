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
use crate::{Range, Span};

use std::convert::TryFrom;

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

        writeln!(formatter, " {}", self.chunk.as_str())?;

        // underline the relevant part with ^^^^^
        let marker_size = if self.span.end.line == self.span.start.line {
            // column bounds are inclusive, so for a correct length we need to add + 1
            self.span.end.column.saturating_sub(self.span.start.column) + 1
        } else {
            self.chunk
                .as_str()
                .chars()
                .count()
                .saturating_sub(self.span.start.column)
        };

        // if the offset starts from 0, we still want to continue if the length
        // of the marker is at least length 1
        let offset = self.range.start;

        if marker_size > 0 {
            context_marker
                .apply_to(format!("{:>width$}", "|", width = indent))
                .fmt(formatter)?;
            help.apply_to(format!(" {:>offset$}", "", offset = offset))
                .fmt(formatter)?;
            help.apply_to(format!("{:^>size$}", "", size = marker_size))
                .fmt(formatter)?;
            formatter.write_str("\n")?;
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
7@n"#, indexmap::indexmap! { 0..10 => Span {
    start : LineColumn {
        line: 7usize,
        column: 8usize,
    },
    end : LineColumn {
        line: 9usize,
        column: 4usize,
    }
} });

        let suggestion = Suggestion {
            detector: Detector::Dummy,
            origin: ContentOrigin::TestEntity,
            chunk: &chunk,
            span: Span {
                start: LineColumn { line: 8usize, column: 0 },
                end: LineColumn { line: 8usize, column: 3 }
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
