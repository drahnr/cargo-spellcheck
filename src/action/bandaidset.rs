//! A set of bandaids, which refer the changes required to apply one single suggestion.

use std::convert::TryFrom;

use anyhow::{anyhow, Error, Result};
use log::trace;

use crate::suggestion::Suggestion;
use crate::{LineColumn, Span};

use super::BandAid;

/// A set of `BandAids` for an accepted suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstAidKit {
    /// All `Bandaids` in this kit constructed from the replacement of a suggestion,
    /// each bandaid covers at most one complete line
    pub bandaids: Vec<BandAid>,
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

impl From<Vec<BandAid>> for FirstAidKit {
    fn from(bandaids: Vec<BandAid>) -> Self {
        Self { bandaids }
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
                end: LineColumn {
                    line: span_lines
                        .next()
                        .ok_or_else(|| anyhow!("Span must cover at least one line"))?,
                    column: *end_of_line
                        .first()
                        .expect("Suggestions have existential coverage. qed"),
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
                    // If we don't use `line` for the `end.line`, we won't have multiline bandaids, but the inital lines
                    // that were longer than the replacement will remain in the content
                    Span {
                        start: crate::LineColumn { line, column: 0 },
                        end: span.end,
                    }
                };
                let bandaid = BandAid::try_from((replacement.to_string(), span_line))?;
                bandaids.push(bandaid);
            }
            Ok(Self { bandaids })
        } else {
            FirstAidKit::try_from((replacement, &suggestion.span))
        }
    }
}

impl TryFrom<(&String, &Span)> for FirstAidKit {
    type Error = Error;

    fn try_from((replacement, span): (&String, &Span)) -> Result<Self> {
        if span.is_multiline() {
            anyhow::bail!("Can't construct `FirstAidKit` from multiline span only")
        } else {
            let bandaid = BandAid::try_from((replacement.to_string(), *span))?;
            Ok(Self::from(bandaid))
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::reflow::{Reflow, ReflowConfig};

    use crate::{Checker, ContentOrigin, Documentation};
    use crate::{LineColumn, Span};

    use std::convert::TryInto;

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

    /// Helper macro for spawning reflow based firstaid creations.
    macro_rules! verify_reflow {
        ($content:literal, $bandaids:expr, $n:literal) => {
            let docs = Documentation::from((ContentOrigin::TestEntity, $content));
            let cfg = ReflowConfig {
                max_line_length: $n,
            };
            // Run the reflow checker creating suggestions
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
    fn reflow_tripple_slash_2to2() {
        let expected: &[BandAid] = &[
            BandAid {
                span: (1_usize, 3..80).try_into().unwrap(),
                replacement: " one tousandth time I'm writing a test string. Maybe one could"
                    .to_owned(),
            },
            BandAid {
                span: (2_usize, 3..43).try_into().unwrap(),
                replacement: " automate that. Maybe not. But writing this is annoying".to_owned(),
            },
        ];

        verify_reflow!(
            "/// one tousandth time I'm writing a test string. Maybe one could automate that.
/// Maybe not. But writing this is annoying",
            expected,
            65
        );
    }

    #[test]
    fn reflow_tripple_slash_3to3() {
        let expected: &[BandAid] = &[
            BandAid {
                span: (1_usize, 3..80).try_into().unwrap(),
                replacement: " one tousandth time I'm writing a test string. Maybe one could"
                    .to_owned(),
            },
            BandAid {
                span: (2_usize, 3..61).try_into().unwrap(),
                replacement: " automate that. Maybe not. But writing this is annoying.".to_owned(),
            },
            BandAid {
                span: (3_usize, 3..37).try_into().unwrap(),
                replacement: " However, I don't have a choice now, do I? Come on!".to_owned(),
            },
        ];

        verify_reflow!(
            "/// one tousandth time I'm writing a test string. Maybe one could automate that.
/// Maybe not. But writing this is annoying. However, I don't
/// have a choice now, do I? Come on!",
            expected,
            65
        );
    }

    #[test]
    fn reflow_tripple_slash_1to2() {
        let expected: &[BandAid] = &[BandAid {
            span: (1_usize, 3..77).try_into().unwrap(),
            replacement: " This is the one 💯🗤⛩ time I'm writing
/// a test string with emojis like 😋😋⏪🦀."
                .to_owned(),
        }];

        verify_reflow!(
            "/// This is the one 💯🗤⛩ time I'm writing a test string with emojis like 😋😋⏪🦀.",
            expected,
            40
        );
    }

    #[test]
    fn reflow_tripple_slash_3to2() {
        let expected: &[BandAid] = &[
            BandAid {
                span: Span {
                    start: LineColumn {
                        line: 1usize,
                        column: 3usize,
                    },
                    end: LineColumn {
                        line: 1usize,
                        column: 38usize,
                    },
                },
                replacement: " Possible __ways__ to run __rustc__ and request various".into(),
            },
            BandAid {
                span: Span {
                    start: LineColumn {
                        line: 2usize,
                        column: 3usize,
                    },
                    end: LineColumn {
                        line: 3usize,
                        column: 25usize,
                    },
                },
                replacement: " parts of LTO described in 3 lines.".into(),
            },
        ];

        verify_reflow!(
            "/// Possible __ways__ to run __rustc__
/// and request various parts of LTO
/// described in 3 lines.",
            expected,
            60
        );
    }

    #[test]
    fn reflow_tripple_slash_2to1() {
        let expected: &[BandAid] = &[BandAid {
            span: Span {
                start: LineColumn {
                    line: 1usize,
                    column: 7usize,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 21usize,
                },
            },
            replacement: "Possibilities are endless, described in 2 lines.".to_owned(),
        }];

        verify_reflow!(
            r###"#[doc="Possibilities are endless,
described in 2 lines."]"###,
            expected,
            80
        );
    }

    #[test]
    fn reflow_hash_doc_eq_1to2() {
        let expected: &[BandAid] = &[BandAid {
            span: Span {
                start: LineColumn {
                    line: 1usize,
                    column: 7usize,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 25usize,
                },
            },
            replacement: "Possibilities are endless, needless to say.".to_owned(),
        }];
        // TODO design decision: do we want to merge these into one, or produce one per line?
        // Imho we should start with implementing one, but ultimately support both approaches.
        verify_reflow!(
            r###"#[doc="Possibilities are
       endless, needless to say."]"###,
            expected,
            30
        );
    }

    #[test]
    fn reflow_hash_doc_eq_2to2() {
        let expected: &[BandAid] = &[BandAid {
            span: Span {
                start: LineColumn {
                    line: 1usize,
                    column: 7usize,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 32usize,
                },
            },
            replacement: r#"Possibilities are
       endless, needless to say."#
                .to_owned(),
        }];

        // TODO design decision: do we want to merge these into one, or produce one per line?
        // Imho we should start with implementing one, but ultimately support both approaches.
        verify_reflow!(
            r###"#[doc="Possibilities are
       endless, needless to say."]"###,
            expected,
            30
        );
    }

    #[test]
    fn reflow_hash_doc_eq_2to1() {
        let expected: &[BandAid] = &[BandAid {
            span: Span {
                start: LineColumn {
                    line: 1usize,
                    column: 7usize,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 25usize,
                },
            },
            replacement: "Possibilities are endless, described in 2 lines.".to_owned(),
        }];

        verify_reflow!(
            r###"#[doc="Possibilities are endless,
       described in 2 lines."]"###,
            expected,
            60
        );
    }

    // TODO checks for all doc variants:
    //
    // * `#[doc="x"]`
    // * `#[doc=r"x"]`
    // * `#[doc=r#"x"#]`
    // * `#[doc=r##"x"##]`
    // * `#[doc=r###"x"###]`
    // * `#[doc=r####"x"####]`
    // * `#[doc=r#####"x"#####]` (more are very very uncommon)
    // * `//! x`
    // * `/*! x */`
    // * `/// x`
}
