//! The desired error output should look like this:
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

use super::extractor::Documentation;

use anyhow::{anyhow, Result};

use languagetool::{LanguageTool, Request};

const HUNSPELL_AFF_DIR: &str = "/usr/share/myspell/";
const HUNSPELL_DIC_DIR: &str = "/usr/share/myspell/";

use hunspell::Hunspell;

pub use proc_macro2::LineColumn;

/// The suggestion of span relative within a source file.
pub struct SuggestionSpan<'a> {
    path: &'a Path,
    start: LineColumn,
    end: LineColumn,
}

impl<'a, 'b> From<(&'b Path, proc_macro2::Span)> for SuggestionSpan<'a>
where
    'b: 'a,
{
    fn from(tup: (&'b Path, proc_macro2::Span)) -> Self {
        let (path, span) = tup;
        Self {
            path,
            start: span.start(),
            end: span.end(),
        }
    }
}

/// A suggestion for certain offending span.
#[derive(Clone, Debug)]
pub struct Suggestion {
    /// Reference to the file location
    pub path: PathBuf,
    pub span: RelativeSpan,
    pub replacements: Vec<String>,
}

use std::fmt;

impl fmt::Display for Suggestion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use console::Style;

        let highlight = Style::new().bold().white();
        let error = Style::new().bold().red();
        let arrow_marker = Style::new().blue();
        let context_marker = Style::new().bold().blue();
        let replacement = Style::new().white();

        let line_number_digit_count = dbg!(self.span.start.line.to_string()).len();
        let indent = 3 + line_number_digit_count;

        error.apply_to("error").fmt(formatter)?;
        highlight.apply_to(": spellcheck").fmt(formatter)?;
        formatter.write_str("\n");

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
        formatter.write_str("\n");
        context_marker
            .apply_to(format!(
                "{:>width$} |",
                self.span.start.line,
                width = indent - 2,
            ))
            .fmt(formatter)?;

        writeln!(formatter, " {}", "XX TODO XXX EXTRACT THE REAL SENTENCE")?;
        context_marker
            .apply_to(format!("{:>width$}", "|", width = indent))
            .fmt(formatter)?;

        let replacement = match self.replacements.len() {
            0 => String::new(),
            1 => format!(" - {}", self.replacements[0]),
            2 => format!(" - {} or {}", self.replacements[0], self.replacements[1]),
            n => {
                let last = &self.replacements[n - 1];
                let joined = self.replacements[..n - 1].join(", ");
                format!(" - {}, or {}", joined, last)
            }
        };

        error.apply_to(replacement).fmt(formatter)?;
        formatter.write_str("\n");
        context_marker
            .apply_to(format!("{:>width$}", "|", width = indent))
            .fmt(formatter)?;
        formatter.write_str("\n")
    }
}

impl Suggestion {
    /// Show with style
    pub fn show(&self) -> anyhow::Result<()> {
        println!("{}", self);
        Ok(())
    }
}

/// Relative span in relation
/// to the beginning of a doc comment.
#[derive(Clone, Debug, Copy)]
// TODO ,Eq,PartialEq,PartialOrd,Ord
pub struct RelativeSpan {
    pub start: LineColumn,
    pub end: LineColumn,
}

/// rturns absolute offsets
fn tokenize<'a>(literal: &'a proc_macro2::Literal) -> Vec<(String, RelativeSpan)> {
    let mut start = LineColumn { line: 0, column: 0 };
    let mut end = LineColumn { line: 0, column: 0 };
    let mut column = 0usize;
    let mut row = 0usize;
    let mut started = false;
    let mut linear_start = 0usize;
    let mut linear_end = 0usize;
    let s = literal.to_string();
    let mut bananasplit = Vec::with_capacity(32);
    for (kar_idx, kar) in s.char_indices() {
        if kar.is_whitespace() {
            linear_end = kar_idx;
            end = LineColumn {
                line: row,
                column: column,
            };
            if started {
                // shift by abs offset
                if literal.span().start().line == 0 {
                    start.column += literal.span().start().column;
                }
                start.line += literal.span().start().line;

                if literal.span().start().line == 0 {
                    end.column += literal.span().start().column;
                }
                end.line += literal.span().start().line;

                bananasplit.push((
                    s[linear_start..linear_end].to_string(),
                    RelativeSpan { start, end },
                ));
            }
            started = false;
            if kar == '\n' {
                column = 0;
                row += 1;
            }
        } else {
            if !started {
                linear_start = kar_idx;
                start = LineColumn {
                    line: row,
                    column: column,
                };
                started = true;
            }
        }
    }
    dbg!(bananasplit)
}

fn tokenize_literals<'a>(literals: &'a [proc_macro2::Literal]) -> Vec<(String, RelativeSpan)> {
    literals
        .iter()
        .fold(Vec::with_capacity(128), |mut acc, literal| {
            acc.append(&mut tokenize(&literal));
            acc
        })
}

pub(crate) fn check<'a>(docu: &'a Documentation) -> Result<Vec<Suggestion>> {
    let grammar: bool = false;
    let spelling: bool = true;
    let mut corrections = Vec::<Suggestion>::with_capacity(128);

    let literal_to_string = |literal: &proc_macro2::Literal| -> String { format!("{}", literal) };
    let literals_to_string = |literals: &[proc_macro2::Literal]| -> String {
        literals
            .into_iter()
            .map(literal_to_string)
            .collect::<Vec<String>>()
            .join("\n")
    };

    if grammar {
        // TODO make configurable
        // FIXME properly handle
        let url = "https://127.0.0.1:1337";
        let lt = LanguageTool::new(url)?;
        let mut suggestions = docu.iter().try_fold::<Vec<Suggestion>, _, Result<_>>(
            Vec::with_capacity(128),
            |mut acc, (path, literals)| {
                let text: String = literals_to_string(literals.as_slice());
                // let text = text.as_str();
                let req = Request::new(text, "en-US".to_owned());
                let resp = lt.check(req)?;
                let _ = dbg!(resp);
                // TODO convert response to offsets and errors
                acc.push(Suggestion {
                    span: RelativeSpan {
                        start: LineColumn {
                            line: 0usize,
                            column: 1337usize,
                        },
                        end: LineColumn {
                            line: 0usize,
                            column: 1337usize,
                        },
                    },
                    path: PathBuf::from(path),
                    replacements: vec![],
                });
                Ok(acc)
            },
        )?;

        corrections.append(&mut suggestions);
    }

    if spelling {
        // TODO make configurable
        let lang = "en_US";
        let mut aff_file = PathBuf::from(HUNSPELL_AFF_DIR).join(lang);
        aff_file.set_extension("aff");
        let mut dic_file = PathBuf::from(HUNSPELL_DIC_DIR).join(lang);
        dic_file.set_extension("dic");

        let hunspell = Hunspell::new(
            aff_file.to_str().expect(".aff file must exist"),
            dic_file.to_str().expect(".dic file must exist"),
        );
        let mut suggestions =
            docu.iter()
                .fold(Vec::with_capacity(128), |mut acc, (path, literals)| {
                    // FIXME literals should be passed directly to tokenize to allow
                    // for correct span calculation
                    for (word, rspan) in tokenize_literals(literals) {
                        let word = word.as_str();
                        if !hunspell.check(word) {
                            let replacements = hunspell.suggest(word);
                            // FIXME translate the rspan back to
                            acc.push(Suggestion {
                                span: rspan,
                                path: PathBuf::from(path),
                                replacements,
                            })
                        }
                    }
                    acc
                });

        corrections.append(&mut suggestions);
    }
    // TODO sort spans by file and line + column
    Ok(corrections)
}
