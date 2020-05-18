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

use super::checker::RelativeSpan;

pub use proc_macro2::LineColumn;

/// The suggestion of span relative within a source file.
// pub struct SuggestionSpan<'a> {
//     path: &'a Path,
//     start: LineColumn,
//     end: LineColumn,
// }

// impl<'a, 'b> From<(&'b Path, proc_macro2::Span)> for SuggestionSpan<'a>
// where
//     'b: 'a,
// {
//     fn from(tup: (&'b Path, proc_macro2::Span)) -> Self {
//         let (path, span) = tup;
//         Self {
//             path,
//             start: span.start(),
//             end: span.end(),
//         }
//     }
// }

/// A suggestion for certain offending span.
#[derive(Clone, Debug)]
pub struct Suggestion<'s> {
    /// Reference to the file location.
    pub path: PathBuf,
    /// Literal we are referencing.
    pub literal: &'s proc_macro2::Literal, // TODO merge adjacent literals
    /// The span (absolute!) of where it is supposed to be used. TODO make this relative towards the literal.
    pub span: RelativeSpan,
    /// Fix suggestions, might be words or the full sentence.
    pub replacements: Vec<String>,
}

use std::fmt;

impl<'s> fmt::Display for Suggestion<'s> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use console::Style;

        let highlight = Style::new().bold().white();
        let error = Style::new().bold().red();
        let arrow_marker = Style::new().blue();
        let context_marker = Style::new().bold().blue();
        let fix = Style::new().green();
        let help = Style::new().yellow().bold();

        let line_number_digit_count = dbg!(self.span.start.line.to_string()).len();
        let indent = 3 + line_number_digit_count;

        error.apply_to("error").fmt(formatter)?;
        highlight.apply_to(": spellcheck").fmt(formatter)?;
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

        // TODO spans must be adjusted accordingly! probably impossible
        // trimming must be done one after the other and size must be kept track of!
        let literal_str = self.literal.to_string();
        let orig_len = literal_str.len();
        let literal_str =
            literal_str.trim_start_matches(|c: char| c.is_whitespace() || c == '\n' || c == '"');
        let shift_left = orig_len - literal_str.len();
        let literal_str =
            literal_str.trim_end_matches(|c: char| c.is_whitespace() || c == '\n' || c == '"');
        let _tail_cut = orig_len - shift_left - literal_str.len();

        writeln!(formatter, " {}", literal_str)?;

        // underline the relevant part with ^^^^^

        let size = if self.span.end.line == self.span.start.line {
            self.span.end.column.saturating_sub(self.span.start.column)
        } else {
            literal_str.len().saturating_sub(self.span.start.column)
        };

        if size > 0 && shift_left <= self.span.start.column {
            // TODO should have never ended up in here
            // TODO trim must be done before hands
            context_marker
                .apply_to(format!("{:>width$}", "|", width = indent))
                .fmt(formatter)?;
            help.apply_to(format!(
                " {:>offset$}",
                "",
                offset = self.span.start.column - shift_left
            ))
            .fmt(formatter)?;
            help.apply_to(format!("{:^>size$}", "", size = size))
                .fmt(formatter)?;
            formatter.write_str("\n")?;
        }

        context_marker
            .apply_to(format!("{:>width$}", "|", width = indent))
            .fmt(formatter)?;

        let replacement = match self.replacements.len() {
            0 => String::new(),
            1 => format!(" - {}", fix.apply_to(&self.replacements[1])),
            2 => format!(
                " - {} or {}",
                fix.apply_to(&self.replacements[0]).to_string(),
                fix.apply_to(&self.replacements[1]).to_string()
            ),
            n => {
                let last = fix.apply_to(&self.replacements[n - 1]).to_string();
                let joined = self.replacements[..n - 1]
                    .iter()
                    .map(|x| fix.apply_to(x).to_string())
                    .collect::<Vec<String>>()
                    .as_slice()
                    .join(", ");
                format!(" - {}, or {}", joined, last)
            }
        };

        error.apply_to(replacement).fmt(formatter)?;
        formatter.write_str("\n")?;
        context_marker
            .apply_to(format!("{:>width$}", "|", width = indent))
            .fmt(formatter)?;
        formatter.write_str("\n")
    }
}

impl<'s> Suggestion<'s> {
    /// Show with style
    pub fn show(&self) -> anyhow::Result<()> {
        println!("{}", self);
        Ok(())
    }
}
