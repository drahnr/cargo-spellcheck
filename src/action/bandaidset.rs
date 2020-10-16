//! A set of bandaids, which refer the changes required to apply one single suggestion.

use std::convert::TryFrom;

use anyhow::{anyhow, bail, Error, Result};
use log::trace;

use crate::CheckableChunk;
use crate::{CommentVariant, LineColumn, Replacement, Span};

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
            bail!("Can't construct `FirstAidKit` from single-line span only")
        } else {
            // Is only used for custom replacement
            let bandaid = BandAid::Replacement(*span, replacement, CommentVariant::Unknown, 0);
            Ok(Self::from(bandaid))
        }
    }
}

impl TryFrom<(&Span, Replacement, CommentVariant)> for FirstAidKit {
    type Error = Error;

    fn try_from(
        (span, replacement, variant): (&Span, Replacement, CommentVariant),
    ) -> Result<Self> {
        if span.is_multiline() {
            bail!("Can't construct `FirstAidKit` from single-line span only")
        } else {
            let bandaid = BandAid::Replacement(
                *span,
                replacement.content,
                variant,
                *replacement.indentation.first().unwrap(),
            );
            Ok(Self::from(bandaid))
        }
    }
}

impl FirstAidKit {
    /// Extract a set of bandaids by means of parsing the chunk
    pub fn load_from(chunk: &CheckableChunk, span: Span, replacement: Replacement) -> Result<Self> {
        trace!(
            "proc_macro literal span of doc comment: ({},{})..({},{})",
            span.start.line,
            span.start.column,
            span.end.line,
            span.end.column
        );

        if span.is_multiline() || replacement.content.lines().count() > 1 {
            let mut replacement_lines = replacement.content.lines().peekable();
            let mut span_lines = (span.start.line..=span.end.line).peekable();
            let mut bandaids: Vec<BandAid> = Vec::new();
            let mut indentations = replacement.indentation.iter();
            let first_line = replacement_lines
                .next()
                .ok_or_else(|| anyhow!("Replacement must contain at least one line"))?
                .to_owned();

            let line_lengths = chunk.extract_line_lengths()?;
            let mut line_lengths = line_lengths.iter();

            let first_span = Span {
                start: span.start,
                end: LineColumn {
                    line: span_lines.next().ok_or_else(|| {
                        anyhow!("Span used for a `Bandaid` has minimum existential size. qed")
                    })?,
                    column: *line_lengths
                        .next()
                        .ok_or_else(|| anyhow!("Chunk covers one line. qed"))?,
                },
            };
            // bandaid for first line
            bandaids.push(BandAid::Replacement(
                first_span,
                first_line,
                chunk.variant(),
                *indentations.next().unwrap(),
            ));

            // process all subsequent lines
            while let Some(replacement) = replacement_lines.next() {
                let bandaid = if let Some(line) = span_lines.next() {
                    // Replacement covers a line in original content

                    let indent = *indentations.next().unwrap();

                    let span = Span {
                        start: crate::LineColumn {
                            line,
                            column: indent + chunk.variant().prefix(),
                        },
                        end: crate::LineColumn {
                            line,
                            column: *line_lengths
                                .next()
                                .ok_or_else(|| anyhow!("Chunk covers relevant lines. qed"))?,
                        },
                    };
                    BandAid::Replacement(span, replacement.to_string(), chunk.variant(), indent)
                } else {
                    // Original content is shorter than replacement
                    let insertion = LineColumn {
                        // Inections are inserted __before__ the specified line, hence +1
                        line: span.end.line + 1,
                        column: 0,
                    };
                    BandAid::Injection(
                        insertion,
                        replacement.to_string(),
                        chunk.variant(),
                        *indentations.next().unwrap(),
                    )
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
                        column: *line_lengths
                            .next()
                            .ok_or_else(|| anyhow!("Chunk covers relevant lines. qed"))?,
                    },
                };
                let bandaid = BandAid::Deletion(span);
                bandaids.push(bandaid);
            }

            Ok::<_, Error>(Self { bandaids })
        } else {
            FirstAidKit::try_from((&span, replacement, chunk.variant()))
        }
    }
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::chyrp_up;
    use crate::reflow::{Reflow, ReflowConfig};
    use crate::Suggestion;

    use crate::{Checker, CommentVariant, ContentOrigin, Documentation};
    use crate::{LineColumn, Span};

    use std::convert::TryInto;

    #[test]
    fn firstaid_from_replacement() {
        const REPLACEMENT: &'static str = "the one thousandth time I'm writing";

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

        // try_from() based on replacement and span has no information about comment variant
        let expected: &[BandAid] = &[BandAid::Replacement(
            (1_usize, 16..45).try_into().unwrap(),
            "the one thousandth time I'm writing".to_owned(),
            CommentVariant::Unknown,
            0_usize,
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
            // assert_eq!(suggestions.len(), 1);
            let suggestion = suggestions.first().expect("Contains one suggestion. qed");

            let replacement = suggestion
                .replacements
                .get(0_usize)
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
                (1_usize, 3..81).try_into().unwrap(),
                " one tousandth time I'm writing a test string. Maybe one could".to_owned(),
                CommentVariant::TripleSlash,
                0_usize,
            ),
            BandAid::Replacement(
                (2_usize, 3..44).try_into().unwrap(),
                " automate that. Maybe not. But writing this is annoying".to_owned(),
                CommentVariant::TripleSlash,
                0_usize,
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
                (1_usize, 3..81).try_into().unwrap(),
                " one tousandth time I'm writing a test string. Maybe one could".to_owned(),
                CommentVariant::TripleSlash,
                0_usize,
            ),
            BandAid::Replacement(
                (2_usize, 3..62).try_into().unwrap(),
                " automate that. Maybe not. But writing this is annoying.".to_owned(),
                CommentVariant::TripleSlash,
                0_usize,
            ),
            BandAid::Replacement(
                (3_usize, 3..38).try_into().unwrap(),
                " However, I don't have a choice now, do I? Come on!".to_owned(),
                CommentVariant::TripleSlash,
                0_usize,
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
                (1_usize, 3..78).try_into().unwrap(),
                " This is the one 💯🗤⛩ time I'm writing".to_owned(),
                CommentVariant::TripleSlash,
                0_usize,
            ),
            BandAid::Injection(
                LineColumn { line: 2, column: 0 },
                " a test string with emojis like 😋😋⏪🦀.".to_owned(),
                CommentVariant::TripleSlash,
                0_usize,
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
                (1_usize, 3..39).try_into().unwrap(),
                " Possible __ways__ to run __rustc__ and request various".into(),
                CommentVariant::TripleSlash,
                0_usize,
            ),
            BandAid::Replacement(
                (2_usize, 3..37).try_into().unwrap(),
                " parts of LTO described in 3 lines.".into(),
                CommentVariant::TripleSlash,
                0_usize,
            ),
            BandAid::Deletion((3_usize, 0..26).try_into().unwrap()),
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
    fn reflow_tripple_slash_11to22() {
        let expected: &[BandAid] = &[
            BandAid::Replacement(
                (1_usize, 3..72).try_into().unwrap(),
                " Possible __ways__ to run __rustc__".into(),
                CommentVariant::TripleSlash,
                0_usize,
            ),
            BandAid::Injection(
                LineColumn { line: 2, column: 0 },
                " and request various parts of LTO".into(),
                CommentVariant::TripleSlash,
                0_usize,
            ),
        ];

        verify_reflow!(
            "/// Possible __ways__ to run __rustc__ and request various parts of LTO
///
/// A third line which also is gonna be broken up.",
            expected,
            40
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
                        column: 9_usize + 43,
                    },
                },
                "Possibilities are".to_owned(),
                CommentVariant::MacroDocEq(2),
                7_usize,
            ),
            BandAid::Injection(
                LineColumn { line: 2, column: 0 },
                "endless, needless to say.".into(),
                CommentVariant::MacroDocEq(2),
                7_usize,
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
                        line: 1_usize,
                        column: 9_usize,
                    },
                    end: LineColumn {
                        line: 1_usize,
                        column: 9 + 35_usize,
                    },
                },
                r#"Possibilities are endless,"#.to_owned(),
                CommentVariant::MacroDocEq(2),
                7_usize,
            ),
            BandAid::Replacement(
                (2_usize, 9..15).try_into().unwrap(),
                "needless to say.".into(),
                CommentVariant::MacroDocEq(2),
                7_usize,
            ),
        ];

        // TODO design decision: do we want to merge these into one, or produce one per line?
        // Imho we should start with implementing one, but ultimately support both approaches.
        let content = chyrp_up!(["Possibilities are endless, needless", "       to say."]);
        dbg!(&content);
        verify_reflow!(content, expected, 35);
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
                        column: 9 + 26usize,
                    },
                },
                "Possibilities are endless, described in 2 lines.".to_owned(),
                CommentVariant::MacroDocEq(2),
                7_usize,
            ),
            BandAid::Deletion((2_usize, 0..29).try_into().unwrap()),
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
