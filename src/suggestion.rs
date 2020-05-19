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

use super::checker::Span;

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


#[derive(Clone,Debug)]
/// A litteral with meta info where the first and list whitespace may be found.
pub struct AnnotatedLiteral<'l> {
    /// The literal which this annotates to.
    pub literal: &'l proc_macro2::Literal,
    /// the complete rendered string including post and pre.
    pub rendered: String,
    /// Whitespace prefix len + 1
    pub pre: usize,
    /// Whitespace postfix len + 1
    pub post: usize,
    /// Length without pre and post
    /// if all whitespace, this is zer0 and the sum of pre+post is 2x len
    pub len: usize,
}

impl<'l> From<&'l proc_macro2::Literal> for AnnotatedLiteral<'l> {
    fn from(literal: &'l proc_macro2::Literal) -> Self {
        let rendered = literal.to_string();
        let scrap = |c: &'_ char| -> bool { c.is_whitespace() };
        let pre = 1 + rendered.chars().take_while(scrap).count();
        let post = 1 + rendered.chars().rev().take_while(scrap).count();
        Self {
            len: if rendered.len() > pre + post {rendered.len() - pre - post} else { 0 },
            literal,
            rendered,
            pre,
            post,
        }
    }
}

impl<'l> std::ops::Deref for AnnotatedLiteral<'l> {
    type Target = proc_macro2::Literal;
    fn deref(&self) -> &Self::Target {
        self.literal
    }
}

impl<'l> AnnotatedLiteral<'l> {
    pub fn as_str(&self) -> &str {
        &self.rendered.as_str()[self.pre..(self.pre+self.len)]
    }
}

use enumflags2::BitFlags;



/// Bitflag of available checkers by compilation / configuration.
#[derive(Debug, Clone, Copy, BitFlags)]
#[repr(u8)]
pub enum Detector {
    Hunspell = 0b0001,
    LanguageTool = 0b0010,
}

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
#[derive(Clone, Debug)]
pub struct Suggestion<'s> {
    pub detector: Detector,
    /// Reference to the file location.
    pub path: PathBuf,
    /// Literal we are referencing.
    pub literal: AnnotatedLiteral<'s>, // TODO merge adjacent literals
    /// The span (absolute!) of where it is supposed to be used. TODO make this relative towards the literal.
    pub span: Span,
    /// Fix suggestions, might be words or the full sentence.
    pub replacements: Vec<String>,
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

        // FIXME this needs some more thought
        // and mostly works since it currently does not contain
        // multilines
        let marker_size = if self.span.end.line == self.span.start.line {
            self.span.end.column.saturating_sub(self.span.start.column)
        } else {
            self.literal.len.saturating_sub(self.span.start.column)
        };

        if marker_size > 0 && self.literal.pre <= self.span.start.column {
            // TODO should have never ended up in here
            // TODO trim must be done before hands
            context_marker
                .apply_to(format!("{:>width$}", "|", width = indent))
                .fmt(formatter)?;
            help.apply_to(format!(
                " {:>offset$}",
                "",
                offset = self.span.start.column - self.literal.pre
            ))
            .fmt(formatter)?;
            help.apply_to(format!("{:^>size$}", "", size = marker_size))
                .fmt(formatter)?;
            formatter.write_str("\n")?;
        } else {
            log::trace!("marker_size={} [{}|{}|{}] literal {{ {:?} .. {:?} }}", marker_size, self.literal.pre, self.literal.len, self.literal.post,
            self.literal.span().start(),
            self.literal.span().end(),
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
            },
            _n => {
                let joined = self.replacements[..=6]
                    .iter()
                    .map(|x| fix.apply_to(x).to_string())
                    .collect::<Vec<String>>()
                    .as_slice()
                    .join(", ");

                let remaining = self.replacements.len()-6;
                let remaining = fix.apply_to(format!("{}", remaining)).to_string();
                format!(" - {}, or one of {} others", joined, remaining)
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
