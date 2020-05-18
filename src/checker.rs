use std::path::PathBuf;

use super::extractor::Documentation;
use super::suggestion::{Suggestion,Detector,AnnotatedLiteral};

use anyhow::Result;

pub use proc_macro2::LineColumn;

use languagetool::{LanguageTool, Request};

const HUNSPELL_AFF_DIR: &str = "/usr/share/myspell/";
const HUNSPELL_DIC_DIR: &str = "/usr/share/myspell/";

use hunspell::Hunspell;

/// Relative span in relation
/// to the beginning of a doc comment.
#[derive(Clone, Debug, Copy)]
// TODO ,Eq,PartialEq,PartialOrd,Ord
pub struct RelativeSpan {
    pub start: LineColumn,
    pub end: LineColumn,
}

/// Returns absolute offsets and the data with the token in question.
///
/// Does not handle hypenation yet or partial words at boundaries.
fn tokenize<'a>(literal: &'a proc_macro2::Literal) -> Vec<(String, RelativeSpan)> {
    let mut start = LineColumn { line: 0, column: 0 };
    let mut end;
    let mut column = 0usize;
    let mut line = 0usize;
    let mut started = false;
    let mut linear_start = 0usize;
    let mut linear_end;
    let s = literal.to_string();
    let mut bananasplit = Vec::with_capacity(32);
    for (c_idx, c) in s.char_indices() {
        if c.is_whitespace() {
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

                bananasplit.push(dbg!((
                    s[linear_start..linear_end].to_string(),
                    RelativeSpan { start, end },
                )));
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
    dbg!(bananasplit)
}

/// Tokenize a set of literals.
///
/// Does not handle hyphenation yet!
fn tokenize_literals<'a, 'b>(
    literals: &'a [proc_macro2::Literal],
) -> Vec<(Vec<(String, RelativeSpan)>, &'b proc_macro2::Literal)>
where
    'a: 'b,
{
    literals
        .iter()
        .fold(Vec::with_capacity(128), |mut acc, literal| {
            acc.push((tokenize(&literal), &*literal));
            acc
        })
}

/// Check a full document for violations using the tools we have.
pub(crate) fn check<'a, 's>(docu: &'a Documentation) -> Result<Vec<Suggestion<'s>>>
where
    'a: 's,
{
    let grammar: bool = true;
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
        let url = "http://192.168.1.127:8010";
        let lt = LanguageTool::new(url)?;
        let mut suggestions = docu.iter().try_fold::<Vec<Suggestion>, _, Result<_>>(
            Vec::with_capacity(128),
            |mut acc, (path, literals)| {
                let text: String = literals_to_string(literals.as_slice());
                // let text = text.as_str();
                let req = Request::new(text, "en-US".to_owned());
                let resp = lt.check(req)?;
                if let Some(software) = resp.software {
                    log::trace!("sw: {:?}", software);
                }
                if let Some(matches) = resp.matches {
                    for item in matches {
                        // log::trace!("item.context: {:?}", item.context);
                        // log::trace!("item.message: {:?}", item.message);
                        // log::trace!("item.short_message: {:?}", item.short_message);
                        // log::trace!("item.rule: {:?}", item.rule);
                        // log::trace!("item.replacements: {:?}", item.rule);
                        // TODO convert response to offsets and errors with the matching literal
                        let start = LineColumn {
                            line: item.offset as usize,  // FIXME wrong
                            column: 0usize,  // FIXME wrong
                        };
                        let end = LineColumn {
                            line: (item.offset + item.length) as usize,  // FIXME wrong
                            column: item.length as usize, // FIXME wrong
                        };
                        acc.push(Suggestion {
                            detector: Detector::LanguageTool,
                            span: RelativeSpan {
                                start,
                                end,
                            },
                        path: PathBuf::from(path),
                        replacements: item.replacements.into_iter().filter_map(|x| {
                            x.value
                        }).collect(),
                        literal: AnnotatedLiteral::from(&literals[0]), // FIXME wrong
                        });
                    }
                }
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
                    for (words_with_spans, literal) in tokenize_literals(literals) {
                        for (word, rspan) in words_with_spans {
                            let word = word.as_str();
                            if !hunspell.check(word) {
                                // get rid of single character suggestions
                                let replacements = hunspell
                                    .suggest(word)
                                    .into_iter()
                                    .filter(|x| x.len() != 1)
                                    .collect::<Vec<_>>();
                                // FIXME translate the rspan back to
                                acc.push(Suggestion {
                                    detector: Detector::Hunspell,
                                    span: rspan,
                                    path: PathBuf::from(path),
                                    replacements,
                                    literal: AnnotatedLiteral::from(literal),
                                })
                            }
                        }
                    }
                    acc
                });

        corrections.append(&mut suggestions);
    }
    // TODO sort spans by file and line + column
    Ok(corrections)
}
