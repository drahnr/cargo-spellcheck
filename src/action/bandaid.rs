//! A mistake bandaid.
//!
//! A `BandAid` covers the mistake with a suggested
//! replacement, as picked by the user. It only refers
//! to suggestions on one line.
//! Multi-line suggestions are collected in a `FirstAidKit`.

use crate::span::Span;
use crate::suggestion::Suggestion;
use anyhow::{anyhow, Error, Result};
use log::trace;
use std::convert::TryFrom;

/// A choosen sugestion for a certain span
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BandAid {
    /// a span, where the first line has index 1, columns are base 0
    pub span: Span,
    /// replacement text for the given span
    pub replacement: String,
}

impl From<(String, Span)> for BandAid {
    fn from((replacement, span): (String, Span)) -> Self {
        Self { span, replacement }
    }
}

/// A set of `BandAids` for an accepted suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstAidKit {
    /// All Bandaids in this kit constructed from the replacement of a suggestion,
    /// each bandaid covers at most one complete line
    pub bandaids: Vec<BandAid>,
}

impl FirstAidKit {
    fn new(bandaids: Vec<BandAid>) -> Self {
        Self { bandaids }
    }
}

impl Default for FirstAidKit {
    fn default() -> Self {
        Self {
            bandaids: Vec::new(),
        }
    }
}

impl From<BandAid> for FirstAidKit {
    fn from(bandaid: BandAid) -> Self {
        Self {
            bandaids: vec![bandaid],
        }
    }
}

impl<'s> TryFrom<(&Suggestion<'s>, usize)> for FirstAidKit {
    type Error = Error;
    fn try_from((suggestion, pick_idx): (&Suggestion<'s>, usize)) -> Result<Self> {
        let literal_file_span = suggestion.span;
        trace!(
            "proc_macro literal span of doc comment: ({},{})..({},{})",
            literal_file_span.start.line,
            literal_file_span.start.column,
            literal_file_span.end.line,
            literal_file_span.end.column
        );
        let replacement = suggestion
            .replacements
            .get(pick_idx)
            .ok_or(anyhow::anyhow!("Does not contain any replacements"))?;
        let span = suggestion.span;

        if span.is_multiline() {
            let mut replacement_lines = replacement.lines().peekable();
            let mut span_lines = (span.start.line..=span.end.line).peekable();
            let mut bandaids: Vec<BandAid> = Vec::new();
            let first_line = replacement_lines
                .next()
                .ok_or(anyhow!("Replacement must contain at least one line"))?
                .to_string();
            // get the length of the line in the original content
            let end_of_line: Vec<usize> = suggestion
                .chunk
                .iter()
                .filter_map(|(_k, v)| {
                    if v.start.line == span.start.line {
                        Some(v.end.column)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(end_of_line.len(), 1);

            let first_span = Span {
                start: span.start,
                end: crate::LineColumn {
                    line: span_lines
                        .next()
                        .ok_or(anyhow!("Span must cover at least one line"))?,
                    column: *end_of_line.first().expect("Suggestion must cover one line"),
                },
            };
            // bandaid for first line
            bandaids.push(BandAid::try_from((first_line, first_span))?);

            // process all subsequent lines
            while let Some(replacement) = replacement_lines.next() {
                dbg!(&replacement);
                let line = span_lines
                    .next()
                    // TODO: How can we get rid of lines? E.g., original content had 4 lines, replacement just 2
                    // With this implementation, the original content in the remaining lines are retained
                    .unwrap_or(span.end.line);

                let span_line = if replacement_lines.peek().is_some() {
                    // get the length of the line in the original content
                    let end_of_line: Vec<usize> = suggestion
                        .chunk
                        .iter()
                        .filter_map(|(_, v)| {
                            if v.start.line == line {
                                Some(v.end.column)
                            } else {
                                None
                            }
                        })
                        .collect();
                    assert_eq!(end_of_line.len(), 1);

                    Span {
                        start: crate::LineColumn { line, column: 0 },
                        end: crate::LineColumn {
                            line,
                            column: *end_of_line
                                .first()
                                .expect("Suggestion must cover its own lines"),
                        },
                    }
                } else {
                    // span of last line only covers first column until original span.end
                    // TODO: still results in multiline bandaids if original content had more lines than replacment
                    Span {
                        start: crate::LineColumn { line, column: 0 },
                        end: span.end,
                    }
                };
                let bandaid = BandAid::try_from((replacement.to_string(), span_line))?;
                bandaids.push(bandaid);
            }
            Ok(Self::new(bandaids))
        } else {
            FirstAidKit::try_from((replacement, &suggestion.span))
        }
    }
}

impl TryFrom<(&String, &Span)> for FirstAidKit {
    type Error = Error;

    fn try_from((replacement, span): (&String, &Span)) -> Result<Self> {
        if span.is_multiline() {
            anyhow::bail!("Can't construct FirstAidKit from multiline span only")
        } else {
            let bandaid = BandAid::try_from((replacement.to_string(), *span))?;
            Ok(Self::new(vec![bandaid]))
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::reflow::{Reflow, ReflowConfig};
    use crate::util::load_span_from;
    use crate::{Checker, ContentOrigin, Documentation};
    use crate::{LineColumn, Span};
    use anyhow::anyhow;
    use std::convert::TryInto;
    use std::path::Path;

    /// Extract span from file as String
    /// Helpful to validate bandaids against what's actually in the file
    #[allow(unused)]
    pub(crate) fn load_span_from_file(path: impl AsRef<Path>, span: Span) -> Result<String> {
        let path = path.as_ref();
        let path = path
            .canonicalize()
            .map_err(|e| anyhow!("Failed to canonicalize {}", path.display()).context(e))?;

        let ro = std::fs::OpenOptions::new()
            .read(true)
            .open(&path)
            .map_err(|e| anyhow!("Failed to open {}", path.display()).context(e))?;

        let mut reader = std::io::BufReader::new(ro);

        load_span_from(reader, span)
    }

    #[test]
    fn span_helper_integrity() {
        const SOURCE: &'static str = r#"0
abcde
f
g
hijk
l
"#;

        struct TestSet {
            span: Span,
            expected: &'static str,
        }

        const SETS: &[TestSet] = &[
            TestSet {
                span: Span {
                    start: LineColumn {
                        line: 1usize,
                        column: 0,
                    },
                    end: LineColumn {
                        line: 1usize,
                        column: 0,
                    },
                },
                expected: "0",
            },
            TestSet {
                span: Span {
                    start: LineColumn {
                        line: 2usize,
                        column: 2,
                    },
                    end: LineColumn {
                        line: 2usize,
                        column: 4,
                    },
                },
                expected: "cde",
            },
            TestSet {
                span: Span {
                    start: LineColumn {
                        line: 5usize,
                        column: 0,
                    },
                    end: LineColumn {
                        line: 5usize,
                        column: 1,
                    },
                },
                expected: "hi",
            },
        ];

        for item in SETS {
            assert_eq!(
                load_span_from(SOURCE.as_bytes(), item.span).unwrap(),
                item.expected.to_string()
            );
        }
    }

    #[test]
    fn try_from_string_works() {
        const TEST: &str = include_str!("../../demo/src/main.rs");

        const EXPECTED: &[Span] = &[
            Span {
                start: LineColumn { line: 1, column: 4 },
                end: LineColumn { line: 1, column: 7 },
            },
            Span {
                start: LineColumn { line: 1, column: 9 },
                end: LineColumn { line: 1, column: 9 },
            },
            Span {
                start: LineColumn {
                    line: 1,
                    column: 11,
                },
                end: LineColumn {
                    line: 1,
                    column: 13,
                },
            },
            Span {
                start: LineColumn {
                    line: 1,
                    column: 15,
                },
                end: LineColumn {
                    line: 1,
                    column: 22,
                },
            },
            Span {
                start: LineColumn {
                    line: 1,
                    column: 24,
                },
                end: LineColumn {
                    line: 1,
                    column: 31,
                },
            },
        ];

        crate::checker::tests::extraction_test_body(TEST, EXPECTED);
    }

    #[test]
    fn try_from_raw_string_works() {
        const TEST: &str = include_str!("../../demo/src/lib.rs");
        let fn_with_doc = TEST
            .lines()
            .skip(18)
            .fold(String::new(), |acc, line| acc + line);

        const EXPECTED: &[Span] = &[
            Span {
                start: LineColumn {
                    line: 1,
                    column: 11,
                },
                end: LineColumn {
                    line: 1,
                    column: 14,
                },
            },
            Span {
                start: LineColumn {
                    line: 1,
                    column: 16,
                },
                end: LineColumn {
                    line: 1,
                    column: 17,
                },
            },
            Span {
                start: LineColumn {
                    line: 1,
                    column: 19,
                },
                end: LineColumn {
                    line: 1,
                    column: 21,
                },
            },
            Span {
                start: LineColumn {
                    line: 1,
                    column: 23,
                },
                end: LineColumn {
                    line: 1,
                    column: 26,
                },
            },
            Span {
                start: LineColumn {
                    line: 1,
                    column: 28,
                },
                end: LineColumn {
                    line: 1,
                    column: 32,
                },
            },
            Span {
                start: LineColumn {
                    line: 1,
                    column: 35,
                },
                end: LineColumn {
                    line: 1,
                    column: 38,
                },
            },
            Span {
                start: LineColumn {
                    line: 1,
                    column: 40,
                },
                end: LineColumn {
                    line: 1,
                    column: 43,
                },
            },
            Span {
                start: LineColumn {
                    line: 1,
                    column: 45,
                },
                end: LineColumn {
                    line: 1,
                    column: 47,
                },
            },
            Span {
                start: LineColumn {
                    line: 1,
                    column: 49,
                },
                end: LineColumn {
                    line: 1,
                    column: 53,
                },
            },
            Span {
                start: LineColumn {
                    line: 1,
                    column: 55,
                },
                end: LineColumn {
                    line: 1,
                    column: 57,
                },
            },
            Span {
                start: LineColumn {
                    line: 1,
                    column: 59,
                },
                end: LineColumn {
                    line: 1,
                    column: 61,
                },
            },
        ];

        crate::checker::tests::extraction_test_body(fn_with_doc.as_str(), EXPECTED);
    }

    #[test]
    fn firstaid_from_replacement() {
        const REPLACEMENT: &'static str = "the one tousandth time I'm writing";

        let span = Span {
            start: LineColumn {
                line: 1,
                column: 16,
            },
            end: LineColumn {
                line: 1,
                column: 44,
            },
        };

        let expected: &[BandAid] = &[BandAid {
            span: (1_usize, 16..45).try_into().unwrap(),
            replacement: "the one tousandth time I'm writing".to_owned(),
        }];

        let kit = FirstAidKit::try_from((&REPLACEMENT.to_string(), &span))
            .expect("(String, Span) into FirstAidKit works. qed");
        assert_eq!(kit.bandaids.len(), expected.len());
        dbg!(&kit);
        for (bandaid, expected) in kit.bandaids.iter().zip(expected) {
            assert_eq!(bandaid, expected);
        }
    }

    macro_rules! verify_firstaid {
        ($content:literal, $bandaids:expr, $n:literal) => {
            let docs = Documentation::from((ContentOrigin::TestEntity, $content));
            let cfg = ReflowConfig {
                max_line_length: $n,
            };
            let suggestion_set = Reflow::check(&docs, &cfg).expect("Reflow is working. qed");
            let suggestions: Vec<&Suggestion> = suggestion_set
                .suggestions(&crate::ContentOrigin::TestEntity)
                .collect();
            assert_eq!(suggestions.len(), 1);
            let suggestion = suggestions.first().expect("Contains one suggestion. qed");
            let kit = FirstAidKit::try_from((*suggestion, 0)).expect("Must work");
            assert_eq!(kit.bandaids.len(), $bandaids.len());
            for (bandaid, expected) in kit.bandaids.iter().zip($bandaids) {
                assert_eq!(bandaid, expected);
            }
        };
    }

    #[test]
    fn firstaid_2to2() {
        let expected: &[BandAid] = &[
            BandAid {
                span: (1_usize, 3..80).try_into().unwrap(),
                replacement: " one tousandth time I'm writing a test string. Maybe one could"
                    .to_owned(),
            },
            BandAid {
                span: (2_usize, 0..43).try_into().unwrap(),
                replacement: "/// automate that. Maybe not. But writing this is annoying"
                    .to_owned(),
            },
        ];

        verify_firstaid!(
            "/// one tousandth time I'm writing a test string. Maybe one could automate that.
/// Maybe not. But writing this is annoying",
            expected,
            65
        );
    }

    #[test]
    fn firstaid_3to3() {
        let expected: &[BandAid] = &[
            BandAid {
                span: (1_usize, 3..80).try_into().unwrap(),
                replacement: " one tousandth time I'm writing a test string. Maybe one could"
                    .to_owned(),
            },
            BandAid {
                span: (2_usize, 0..61).try_into().unwrap(),
                replacement: "/// automate that. Maybe not. But writing this is annoying."
                    .to_owned(),
            },
            BandAid {
                span: (3_usize, 0..37).try_into().unwrap(),
                replacement: "/// However, I don't have a choice now, do I? Come on!".to_owned(),
            },
        ];

        verify_firstaid!(
            "/// one tousandth time I'm writing a test string. Maybe one could automate that.
/// Maybe not. But writing this is annoying. However, I don't
/// have a choice now, do I? Come on!",
            expected,
            65
        );
    }

    #[test]
    fn firstaid_1to2() {
        let expected: &[BandAid] = &[BandAid {
            span: (1_usize, 3..77).try_into().unwrap(),
            replacement: " This is the one 💯🗤⛩ time I'm writing
/// a test string with emojis like 😋😋⏪🦀."
                .to_owned(),
        }];

        verify_firstaid!(
            "/// This is the one 💯🗤⛩ time I'm writing a test string with emojis like 😋😋⏪🦀.",
            expected,
            40
        );
    }

    #[test]
    fn firstaid_3to2() {
        let expected: &[BandAid] = &[
            BandAid {
                span: (1_usize, 3..38).try_into().unwrap(),
                replacement: " Possible __ways__ to run __rustc__ and request various".into(),
            },
            BandAid {
                span: (2_usize, 0..37).try_into().unwrap(),
                replacement: "/// parts of LTO described in 3 lines.".into(),
            },
        ];

        verify_firstaid!(
            "/// Possible __ways__ to run __rustc__
/// and request various parts of LTO
/// described in 3 lines.",
            expected,
            60
        );
    }
}
