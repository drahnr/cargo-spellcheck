//! A mistake bandaid.
//!
//! A `BandAid` covers the mistake with a suggested replacement, as picked by
//! the user.

use crate::Span;

/// A chosen suggestion for a certain span
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BandAid {
    /// `String` replaces the content covered by `Span`
    pub content: String,
    /// range which will be replaced
    pub span: Span,
}

impl BandAid {
    /// Check if the bandaid covers `line` which is 1 indexed.
    pub fn covers_line(&self, line: usize) -> bool {
        self.span.covers_line(line)
    }
}

impl From<(String, &Span)> for BandAid {
    fn from((replacement, span): (String, &Span)) -> Self {
        Self {
            content: replacement,
            span: *span,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {

    use crate::util::load_span_from;

    use crate::{LineColumn, Span};

    #[test]
    fn span_helper_integrity() {
        const SOURCE: &str = r#"0
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
                    column: 20,
                },
            },
            Span {
                start: LineColumn {
                    line: 1,
                    column: 22,
                },
                end: LineColumn {
                    line: 1,
                    column: 27,
                },
            },
            Span {
                start: LineColumn {
                    line: 1,
                    column: 28,
                },
                end: LineColumn {
                    line: 1,
                    column: 28,
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
            .take(4)
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
                    column: 18,
                },
            },
            Span {
                start: LineColumn {
                    line: 1,
                    column: 20,
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
                    column: 27,
                },
                end: LineColumn {
                    line: 1,
                    column: 27,
                },
            },
        ];

        crate::checker::tests::extraction_test_body(dbg!(fn_with_doc.as_str()), EXPECTED);
    }
}
