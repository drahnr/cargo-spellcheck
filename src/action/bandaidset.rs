//! A set of bandaids, which refer the changes required to apply one single suggestion.

use std::convert::TryFrom;

use anyhow::{anyhow, bail, Error, Result};
use log::trace;

use crate::CheckableChunk;
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

impl TryFrom<(String, &Span)> for FirstAidKit {
    type Error = Error;

    fn try_from((replacement, span): (String, &Span)) -> Result<Self> {
        if span.is_multiline() {
            bail!("Can't construct `FirstAidKit` from multiline span only")
        } else {
            let bandaid = BandAid::try_from((replacement, *span))?;
            Ok(Self::from(bandaid))
        }
    }
}

impl FirstAidKit {
    /// Extract a set of bandaids by means of parsing the chunk
    pub fn load_from(chunk: &CheckableChunk, span: Span, replacement: String) -> Result<Self> {
        trace!(
            "proc_macro literal span of doc comment: ({},{})..({},{})",
            span.start.line,
            span.start.column,
            span.end.line,
            span.end.column
        );

        if span.is_multiline() || replacement.lines().count() > 1 {
            let mut replacement_lines = replacement.lines().peekable();
            let mut span_lines = (span.start.line..=span.end.line).peekable();
            let mut bandaids: Vec<BandAid> = Vec::new();
            let first_line = replacement_lines
                .next()
                .ok_or_else(|| anyhow!("Replacement must contain at least one line"))?
                .to_owned();

            // get the length of the line in the original content
            let end_of_line: Option<usize> = chunk
                .iter()
                .filter_map(|(_k, v)| {
                    if v.start.line == span.start.line {
                        Some(v.end.column)
                    } else {
                        None
                    }
                })
                .next();

            if end_of_line.is_none() {
                bail!("BUG: Missing end of line terminator")
            }

            let first_span = Span {
                start: span.start,
                end: LineColumn {
                    line: span_lines.next().ok_or_else(|| {
                        anyhow!("Span used for a `Bandaid` has minimum existential size. qed")
                    })?,
                    column: end_of_line.expect("Suggestions have existential coverage. qed"),
                },
            };
            // bandaid for first line
            bandaids.push(BandAid::try_from((first_line, first_span))?);

            // process all subsequent lines
            while let Some(replacement) = replacement_lines.next() {

                let bandaid = if let Some(line) = span_lines.next() {
                    // Replacement covers a line in original content

                    // get the length of the current line in the original content
                    let end_of_line: Vec<usize> = chunk
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

                    let span = Span {
                        start: crate::LineColumn { line, column: 0 },
                        end: crate::LineColumn {
                            line,
                            column: *end_of_line
                                .first()
                                .expect("Suggestion must cover its own lines"),
                        },
                    };
                    BandAid::Replacement(span, replacement.to_string())
                } else {
                    // Original content is shorter than replacement
                    let insertion = LineColumn {
                        line: span.end.line + 1,
                        column: 0,
                    };
                    BandAid::Injection(insertion, replacement.to_string())
                };
                bandaids.push(bandaid);
            }

            // for all remaining lines in the original content, add a deletion
            while let Some(remaining) = span_lines.next() {
                let span = Span {
                    start: LineColumn {
                        line: remaining,
                        column: 0,
                    },
                    end: LineColumn {
                        line: remaining,
                        // TODO: get actual length of original line
                        // column: unimplemented!("Get length of line!"),
                        column: 1,
                    },
                };
                let bandaid = BandAid::Deletion(span);
                bandaids.push(bandaid);
            }

            Ok::<_, Error>(Self { bandaids })
        } else {
            FirstAidKit::try_from((replacement, &span))
        }
    }
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::chyrp_up;
    use crate::reflow::{Reflow, ReflowConfig};
    use crate::Suggestion;

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

        let expected: &[BandAid] = &[BandAid::Replacement(
            (1_usize, 16..45).try_into().unwrap(),
            "the one tousandth time I'm writing".to_owned(),
        )];

        let kit = FirstAidKit::try_from((REPLACEMENT.to_owned(), &span))
            .expect("(String, Span) into FirstAidKit works. qed");
        assert_eq!(kit.bandaids.len(), expected.len());
        dbg!(&kit);
        for (bandaid, expected) in kit.bandaids.iter().zip(expected) {
            assert_eq!(bandaid, expected);
        }
    }

    /// Helper macro for spawning reflow based firstaid creations.
    macro_rules! verify_reflow {
        ($content:expr, $bandaids:expr, $n:literal) => {
            let _ = env_logger::builder()
                .is_test(true)
                .filter(None, log::LevelFilter::Trace)
                .try_init();
            let docs = Documentation::from((ContentOrigin::TestEntityRust, $content));
            let cfg = ReflowConfig {
                max_line_length: $n,
            };
            // Run the reflow checker creating suggestions
            let suggestion_set = Reflow::check(&docs, &cfg).expect("Reflow is working. qed");
            let suggestions: Vec<&Suggestion> = suggestion_set
                .suggestions(&crate::ContentOrigin::TestEntityRust)
                .collect();
            assert_eq!(suggestions.len(), 1);
            let suggestion = suggestions.first().expect("Contains one suggestion. qed");

            let replacement = suggestion
                .replacements
                .get(0usize)
                .expect("Automated test pick is in range. qed");

            let kit =
                FirstAidKit::load_from(&suggestion.chunk, suggestion.span, replacement.to_owned())
                    .expect("Obtaining bandaids from a suggestion succeeds. qed");
            assert_eq!(kit.bandaids.len(), $bandaids.len());
            for (bandaid, expected) in kit.bandaids.iter().zip($bandaids) {
                assert_eq!(bandaid, expected);
            }
        };
    }

    #[test]
    fn reflow_tripple_slash_2to2() {
        let expected: &[BandAid] = &[
            BandAid::Replacement(
                (1_usize, 3..80).try_into().unwrap(),
                " one tousandth time I'm writing a test string. Maybe one could".to_owned(),
            ),
            BandAid::Replacement(
                (2_usize, 0..43).try_into().unwrap(),
                "/// automate that. Maybe not. But writing this is annoying".to_owned(),
            ),
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
            BandAid::Replacement(
                (1_usize, 3..80).try_into().unwrap(),
                " one tousandth time I'm writing a test string. Maybe one could".to_owned(),
            ),
            BandAid::Replacement(
                (2_usize, 0..61).try_into().unwrap(),
                "/// automate that. Maybe not. But writing this is annoying.".to_owned(),
            ),
            BandAid::Replacement(
                (3_usize, 0..37).try_into().unwrap(),
                "/// However, I don't have a choice now, do I? Come on!".to_owned(),
            ),
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
        let expected: &[BandAid] = &[
            BandAid::Replacement(
                (1_usize, 3..77).try_into().unwrap(),
                " This is the one 💯🗤⛩ time I'm writing".to_owned(),
            ),
            BandAid::Injection(
                LineColumn { line: 2, column: 0 },
                "/// a test string with emojis like 😋😋⏪🦀.".to_owned(),
            ),
        ];

        verify_reflow!(
            "/// This is the one 💯🗤⛩ time I'm writing a test string with emojis like 😋😋⏪🦀.",
            expected,
            40
        );
    }

    #[test]
    fn reflow_tripple_slash_3to2() {
        let expected: &[BandAid] = &[
            BandAid::Replacement(
                (1_usize, 3..38).try_into().unwrap(),
                " Possible __ways__ to run __rustc__ and request various".into(),
            ),
            BandAid::Replacement(
                (2_usize, 0..36).try_into().unwrap(),
                "/// parts of LTO described in 3 lines.".into(),
            ),
            BandAid::Deletion((3_usize, 0..1).try_into().unwrap()),
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
    fn reflow_hash_doc_eq_1to2() {
        let expected: &[BandAid] = &[
            BandAid::Replacement(
                Span {
                    start: LineColumn {
                        line: 1_usize,
                        column: 9_usize,
                    },
                    end: LineColumn {
                        line: 1_usize,
                        column: 9_usize + 42,
                    },
                },
                "Possibilities are".to_owned(),
            ),
            BandAid::Injection(
                LineColumn { line: 1, column: 0 },
                "         endless, needless to say.".into(),
            ),
        ];
        // TODO design decision: do we want to merge these into one, or produce one per line?
        // Imho we should start with implementing one, but ultimately support both approaches.
        let content = chyrp_up!(["Possibilities are endless, needless to say."]);
        verify_reflow!(content, expected, 34);
    }

    #[test]
    fn reflow_hash_doc_eq_2to2() {
        let expected: &[BandAid] = &[
            BandAid::Replacement(
                Span {
                    start: LineColumn {
                        line: 1usize,
                        column: 7usize,
                    },
                    end: LineColumn {
                        line: 1usize,
                        column: 32usize,
                    },
                },
                r#"Possibilities are"#.to_owned(),
            ),
            BandAid::Replacement(
                (2_usize, 0..33).try_into().unwrap(),
                "       endless, needless to say".into(),
            ),
        ];

        // TODO design decision: do we want to merge these into one, or produce one per line?
        // Imho we should start with implementing one, but ultimately support both approaches.
        let content = chyrp_up!(["Possibilities are endless, needless", "       to say."]);
        dbg!(&content);
        verify_reflow!(content, expected, 30);
    }

    #[test]
    fn reflow_hash_doc_eq_2to1() {
        let expected: &[BandAid] = &[
            BandAid::Replacement(
                Span {
                    start: LineColumn {
                        line: 1usize,
                        column: 9usize,
                    },
                    end: LineColumn {
                        line: 1usize,
                        column: 27usize,
                    },
                },
                "Possibilities are endless, described in 2 lines.".to_owned(),
            ),
            BandAid::Deletion((2_usize, 0..1).try_into().unwrap()),
        ];

        let content = chyrp_up!(["Possibilities are endless,", "       described in 2 lines."]);
        verify_reflow!(content, expected, 60);
    }

    // TODO checks for all doc variants:
    //
    // * `#[doc="x"]`
    // * `#[doc=r"x"]`
    // * `#[do;c=r#"x"#]`
    // * `#[doc=r##"x"##]`
    // * `#[doc=r###"x"###]`
    // * `#[doc=r####"x"####]`
    // * `#[doc=r#####"x"#####]` (more are very very uncommon)
    // * `//! x`
    // * `/*! x */`
    // * `/// x`
}
