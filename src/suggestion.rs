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

use std::path::PathBuf;

use crate::Span;
use crate::TrimmedLiteralRef;

use enumflags2::BitFlags;
use log::trace;

/// Bitflag of available checkers by compilation / configuration.
#[derive(Debug, Clone, Copy, BitFlags, Eq, PartialEq)]
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
#[derive(Clone)]
pub struct Suggestion<'s> {
    pub detector: Detector,
    /// Reference to the file location.
    pub path: PathBuf,
    /// Literal we are referencing.
    pub literal: TrimmedLiteralRef<'s>,
    /// The span (absolute!) of where it is supposed to be used. TODO make this relative towards the literal.
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

        // @todo this needs some more thought
        // and mostly works since it currently does not contain
        // multilines
        let mut marker_size = if self.span.end.line == self.span.start.line {
            self.span.end.column.saturating_sub(self.span.start.column)
        } else {
            self.literal.len().saturating_sub(self.span.start.column)
        };

        // if the offset starts from 0, we still want to continue if the length
        // of the marker is at least length 1
        let offset = if self.literal.pre() <= self.span.start.column {
            self.span.start.column - self.literal.pre()
        } else {
            trace!("Reducing marker length!");
            // reduce the marker size
            marker_size -= self.span.start.column;
            marker_size -= self.literal.pre();
            // @todo figure out why this is needed, currently this is a hack
            // to make the following work:
            //
            // ```text
            // error: spellcheck(Hunspell)
            //  --> /media/supersonic1t/projects/cargo-spellcheck/./src/tests/fragments.rs:9
            //   |
            // 9 |  Somethign very secret but also not,
            //   |  ^^^^^^^^^
            //   | - Something or Sometime
            //   |
            //   |   Possible spelling mistake found.
            //   |
            // ```
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
        use crate::literalset::TrimmedLiteralRangePrint;
        let printable = TrimmedLiteralRangePrint::from((
            self.literal,
            self.span.start.column..self.span.end.column,
        ));
        write!(formatter, "({}, {:?})", &printable, printable.1)
    }
}
