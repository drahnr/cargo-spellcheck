use crate::span::Span;
use crate::suggestion::Suggestion;
use anyhow::{anyhow, Error, Result};
use log::trace;
use std::convert::TryFrom;

#[doc = r#"A choosen sugestion for a certain span"#]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BandAid {
    /// a span, where the first line has index 1, columns are base 0
    pub span: Span,
    /// replacement text for the given span
    pub replacement: String,
}

impl BandAid {
    pub fn new(replacement: &str, span: &Span) -> Self {
        trace!(
            "proc_macro literal span of doc comment: ({},{})..({},{})",
            span.start.line,
            span.start.column,
            span.end.line,
            span.end.column
        );

        Self {
            span: *span,
            replacement: replacement.to_owned(),
        }
    }
}

impl<'s> TryFrom<(&Suggestion<'s>, usize)> for BandAid {
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

        if let Some(replacement) = suggestion.replacements.iter().nth(pick_idx) {
            Ok(Self::new(replacement.as_str(), &suggestion.span))
        } else {
            Err(anyhow!("Does not contain any replacements"))
        }
    }
}

impl<'s> TryFrom<(Suggestion<'s>, usize)> for BandAid {
    type Error = Error;
    fn try_from((suggestion, pick_idx): (Suggestion<'s>, usize)) -> Result<Self> {
        Self::try_from((&suggestion, pick_idx))
    }
}

impl From<(String, Span)> for BandAid {
    fn from((replacement, span): (String, Span)) -> Self {
        Self { span, replacement }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::checker::{dummy::DummyChecker, Checker};
    use crate::documentation::*;
    use crate::span::Span;
    use log::debug;
    use proc_macro2::{LineColumn, Literal};
    use std::path::PathBuf;
    use std::path::Path;
    use std::io::BufRead;
    use anyhow::bail;

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

    /// Extract span from String as String
    /// Helpful to validate bandaids against what's actually in the string
    // @todo does not handle cross line spans @todo yet
    #[allow(unused)]
    pub(crate) fn load_span_from(mut source: impl BufRead, span: Span) -> Result<String> {
        if span.start.line < 1 {
            bail!("Lines are 1-indexed, can't be less than 1")
        }
        if span.end.line < span.start.line {
            bail!("Line range would be negative, bail")
        }
        if span.end.column < span.start.column {
            bail!("Column range would be negative, bail")
        }
        let line = (&mut source)
            .lines()
            .skip(span.start.line - 1)
            .filter_map(|line| line.ok())
            .next()
            .ok_or_else(||anyhow!("Line not in buffer or invalid"))?;

        let range = span.start.column..(span.end.column+1);
        dbg!(line).get(range).ok_or_else(|| anyhow!("Columns not in line")).map(|s| dbg!(s.to_owned()))
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
            assert_eq!(load_span_from(SOURCE.as_bytes(), item.span).unwrap(), item.expected.to_string());
        }
    }

    // @todo condense and unify with other functions doing the same
    fn try_from_test_body(stream: proc_macro2::TokenStream, expected_spans: &[Span]) {
        let _ = env_logger::builder()
            .filter(None, log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        let origin = ContentOrigin::RustSourceFile(PathBuf::from("dummy/dummy.rs"));
        let d = Documentation::from((origin, stream));

        let suggestion_set = DummyChecker::check(&d, &()).expect("DummyChecker must not fail");

        // one file
        assert_eq!(suggestion_set.len(), 1);
        // with two suggestions
        assert_eq!(suggestion_set.total_count(), expected_spans.len());
        let (_, suggestions) = suggestion_set
            .iter()
            .next()
            .expect("Must have valid 1st suggestion");

        for (index, (suggestion, expected_span)) in
            suggestions.iter().zip(expected_spans.iter()).enumerate()
        {
            debug!("Suggestion: {:?}, index {}", suggestion.replacements, index);
            let bandaid = BandAid::try_from((suggestion, 0)).expect("TryFrom suggestion failed");
            assert_eq!(
                suggestion.replacements,
                vec![format!("replacement_{}", index)]
            );
            assert_eq!(suggestion.span, *expected_span);
            assert_eq!(bandaid.replacement, format!("replacement_{}", index));
            assert_eq!(bandaid.span, *expected_span);
            trace!("Bandaid {:?}", bandaid.replacement);
        }
    }

    #[test]
    #[ignore]
    fn try_from_string_works() {
        const TEST: &str = include_str!("../../demo/src/main.rs");
        let stream =
            syn::parse_str::<proc_macro2::TokenStream>(TEST).expect("Must parse just fine");

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

        try_from_test_body(stream, EXPECTED);
    }

    #[test]
    #[ignore]
    fn try_from_raw_string_works() {
        const TEST: &str = include_str!("../../demo/src/lib.rs");
        let stream =
            syn::parse_str::<proc_macro2::TokenStream>(TEST).expect("Must parse just fine");

        const EXPECTED: &[Span] = &[
            Span {
                start: LineColumn {
                    line: 19,
                    column: 11,
                },
                end: LineColumn {
                    line: 19,
                    column: 14,
                },
            },
            Span {
                start: LineColumn {
                    line: 19,
                    column: 16,
                },
                end: LineColumn {
                    line: 19,
                    column: 17,
                },
            },
            Span {
                start: LineColumn {
                    line: 19,
                    column: 19,
                },
                end: LineColumn {
                    line: 19,
                    column: 21,
                },
            },
            Span {
                start: LineColumn {
                    line: 19,
                    column: 23,
                },
                end: LineColumn {
                    line: 19,
                    column: 26,
                },
            },
            Span {
                start: LineColumn {
                    line: 19,
                    column: 28,
                },
                end: LineColumn {
                    line: 19,
                    column: 32,
                },
            },
            Span {
                start: LineColumn {
                    line: 19,
                    column: 35,
                },
                end: LineColumn {
                    line: 19,
                    column: 38,
                },
            },
            Span {
                start: LineColumn {
                    line: 19,
                    column: 40,
                },
                end: LineColumn {
                    line: 19,
                    column: 43,
                },
            },
            Span {
                start: LineColumn {
                    line: 19,
                    column: 45,
                },
                end: LineColumn {
                    line: 19,
                    column: 47,
                },
            },
            Span {
                start: LineColumn {
                    line: 19,
                    column: 49,
                },
                end: LineColumn {
                    line: 19,
                    column: 53,
                },
            },
            Span {
                start: LineColumn {
                    line: 19,
                    column: 55,
                },
                end: LineColumn {
                    line: 19,
                    column: 57,
                },
            },
            Span {
                start: LineColumn {
                    line: 19,
                    column: 59,
                },
                end: LineColumn {
                    line: 19,
                    column: 61,
                },
            },
        ];

        try_from_test_body(stream, EXPECTED);
    }
}
