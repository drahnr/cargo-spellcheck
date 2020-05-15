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

impl fmt::Display for Suggestion{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use console::Style;

        let highlight = Style::new().white();
        let error = Style::new().bold().red();
        let arrow_marker = Style::new().blue();
        let context_marker = Style::new().bold().blue();
        let replacement = Style::new().white();

        let line_number_digit_count = self.span.start.line.to_string().len();
        let indent = 2 + line_number_digit_count;

        error.apply_to("error").fmt(formatter)?;
        highlight.apply_to("Spelling: Suggestion").fmt(formatter)?;
        write!(
            formatter,
            "{:>indent$}",
            arrow_marker.apply_to("-->").to_string(),
            indent = indent
        )?;
        writeln!(
            formatter,
            " {path}:{line}",
            path = self.path.display(),
            line = self.span.start.line
        )?;
        writeln!(
            formatter,
            "{:^indent$}",
            context_marker.apply_to("| "),
            indent = indent
        )?;
        write!(
            formatter,
            "{:^indent$}",
            context_marker
                .apply_to(format!("| {}", self.span.start.line))
                .to_string(),
            indent = indent,
        )?;

        writeln!(formatter, "{}", "The full sentence should be here")?;
        write!(
            formatter,
            "{:^width$}",
            context_marker.apply_to("| "),
            width = indent
        )?;
        error.apply_to(format!("- {}", "Pick one of..... XXXX")).fmt(formatter)?;
		writeln!(formatter, "{:^width$}|", " ", width = indent)?;
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
#[derive(Clone,Debug,Copy)]
// TODO ,Eq,PartialEq,PartialOrd,Ord
pub struct RelativeSpan {
    pub start: LineColumn,
    pub end: LineColumn,
}

fn tokenize<'a>(full: &'a str) -> impl Iterator<Item = (&'a str, RelativeSpan)>
{
    // FIXME TODO the offset is currently not dealing with \n properly
    full.split_whitespace().map(|sub: &'a str| {
        // assert!(&full as usize >= &sub as usize);
        let offset = 32usize; //unsafe { (full as *const str) as usize - (sub  as *const str) as usize };
        (
            sub,
            RelativeSpan {
                start: LineColumn {
                    line: 0usize,
                    column: offset,
                },
                end: LineColumn {
                    line: 0usize,
                    column: offset + sub.len(),
                },
            },
        )
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
						}
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

        let hunspell = Hunspell::new(aff_file.to_str().expect(".aff file must exist"), dic_file.to_str().expect(".dic file must exist"));
        let mut suggestions =
            docu.iter()
                .fold(Vec::with_capacity(128), |mut acc, (path, literals)| {
					// FIXME literals should be passed directly to tokenize to allow
					// for correct span calculation
                    let text: String = literals_to_string(literals.as_slice());
                    let text = text.as_str();
                    for (word, rspan) in tokenize(text) {
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
