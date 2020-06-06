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

use std::path::{Path, PathBuf};

use crate::Span;
use crate::TrimmedLiteralRef;

use enumflags2::BitFlags;
use log::error;

/// Bitflag of available checkers by compilation / configuration.
#[derive(Debug, Clone, Copy, BitFlags, Eq, PartialEq, Hash)]
#[repr(u8)]
pub enum Detector {
    Hunspell = 0b0001,
    LanguageTool = 0b0010,
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

        writeln!(formatter, " {}", self.literal.as_str())?;

        // underline the relevant part with ^^^^^

        // @todo this needs some more thought once multiline comments pop up
        let mut marker_size = if self.span.end.line == self.span.start.line {
            self.span.end.column.saturating_sub(self.span.start.column)
        } else {
            self.literal.len().saturating_sub(self.span.start.column)
        };

        use crate::literalset::Range;

        let literal_span: Span = Span::from(self.literal.as_ref().literal.span());
        let marker_range_relative: Range = self.span.relative_to(literal_span).expect("Must be ok");

        // if the offset starts from 0, we still want to continue if the length
        // of the marker is at least length 1
        let offset = if self.literal.pre() <= marker_range_relative.start {
            marker_range_relative.start - self.literal.pre()
        } else {
            error!("Reducing marker length! Please report a BUG!");
            // reduce the marker size
            marker_size -= marker_range_relative.start;
            marker_size -= self.literal.pre();
            0
        };

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

    /// Iterater over all references given a path
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
