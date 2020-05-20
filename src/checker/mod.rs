use std::path::PathBuf;

use crate::{
    AnnotatedLiteralRef, ConsecutiveLiteralSet, Detector, Documentation, LineColumn, Span,
    Suggestion,
};

use anyhow::Result;

use log::debug;

#[cfg(feature = "hunspell")]
mod hunspell;
#[cfg(feature = "languagetool")]
mod languagetool;

/// Implementation for a checker
pub(crate) trait Checker {
    fn check<'a, 's>(docu: &'a Documentation) -> Result<Vec<Suggestion<'s>>>
    where
        'a: 's;
}

/// Returns absolute offsets and the data with the token in question.
///
/// Does not handle hypenation yet or partial words at boundaries.
/// Returns the a vector of tokens as part of the string.
fn tokenize<'a>(literal: AnnotatedLiteralRef<'a>) -> Vec<(String, Span)> {
    let mut start = LineColumn { line: 0, column: 0 };
    let mut end;
    let mut column = 0usize;
    let mut line = 0usize;
    let mut started = false;
    let mut linear_start = 0usize;
    let mut linear_end;
    let s = literal.to_string();
    let mut bananasplit = Vec::with_capacity(32);

    // add additional seperator characters for tokenization
    // which is useful to avoid pointless dict lookup failures.
    // TODO extract markdown links first
    let blacklist = "\";:,.?!#(){}[]_-".to_owned();
    let is_ignore_char = |c: char| c.is_whitespace() || blacklist.contains(c);
    for (c_idx, c) in s.char_indices() {
        if is_ignore_char(c) {
            linear_end = c_idx;
            end = LineColumn {
                line: line,
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

                bananasplit.push((s[linear_start..linear_end].to_string(), Span { start, end }));
            }
            started = false;
            if c == '\n' {
                column = 0;
                line += 1;
            }
        } else {
            if !started {
                linear_start = c_idx;
                start = LineColumn {
                    line: line,
                    column: column,
                };
                started = true;
            }
        }
        column += 1;
    }
    bananasplit
}

/// Tokenize a set of literals.
///
/// Does not handle hyphenation yet!
fn tokenize_literals<'a, 'b>(
    literals: &'a [ConsecutiveLiteralSet],
) -> Vec<(Vec<(String, Span)>, AnnotatedLiteralRef<'b>)>
where
    'a: 'b,
{
    literals
        .iter()
        .fold(Vec::with_capacity(128), |mut acc, cls| {
            for literal in cls.literals() {
                acc.push((tokenize(dbg!(literal).into()), literal.into()));
            }
            acc
        })
}

/// Check a full document for violations using the tools we have.
pub fn check<'a, 's>(documentation: &'a Documentation) -> Result<Vec<Suggestion<'s>>>
where
    'a: 's,
{
    let mut corrections = Vec::<Suggestion>::with_capacity(128);

    #[cfg(feature = "languagetool")]
    {
        debug!("Running LanguageTool checks");
        if let Ok(mut suggestions) = self::languagetool::LanguageToolChecker::check(documentation) {
            corrections.append(&mut suggestions);
        }
    }

    #[cfg(feature = "hunspell")]
    {
        debug!("Running Hunspell checks");
        if let Ok(mut suggestions) = self::hunspell::HunspellChecker::check(documentation) {
            corrections.append(&mut suggestions);
        }
    }

    Ok(corrections)
}
