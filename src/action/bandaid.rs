use crate::span::Span;
use crate::suggestion::Suggestion;
use anyhow::{anyhow, Error, Result};
use log::trace;
use std::convert::TryFrom;

#[doc = r#"A choosen sugestion for a certain span"#]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BandAid {
    /// a span, where the first line has index 1, columns are base 1 too
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

        let mut span = span.clone();
        // @todo this is a hack and should be documented better
        // @todo not sure why the offset of two is necessary
        // @todo but it works consistently
        let doc_comment_to_file_offset = 2;
        span.start.column += doc_comment_to_file_offset;
        span.end.column += doc_comment_to_file_offset;
        Self {
            span,
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
mod tests {
    use super::*;
    use crate::checker::{dummy::DummyChecker, Checker};
    use crate::documentation::*;
    use crate::span::Span;
    use log::debug;
    use proc_macro2::{LineColumn, Literal};
    use std::path::PathBuf;

    fn try_from_test_body(content: &'static str, expected_spans: &[Span]) {
        let _ = env_logger::builder()
            .filter(None, log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        let mut d = Documentation::new();
        let dummy_path = PathBuf::from("dummy/dummy.rs");
        d.append_literal(&dummy_path, Literal::string(content));
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
    fn try_from_string_works() {
        const CONTENT: &'static str = "This has four literals";
        const EXPECTED: &[Span] = &[
            Span {
                start: LineColumn { line: 1, column: 1 },
                end: LineColumn { line: 1, column: 4 },
            },
            Span {
                start: LineColumn { line: 1, column: 6 },
                end: LineColumn { line: 1, column: 8 },
            },
            Span {
                start: LineColumn {
                    line: 1,
                    column: 10,
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
        ];

        try_from_test_body(CONTENT, EXPECTED);
    }

    #[test]
    fn try_from_raw_string_works() {
        const CONTENT: &'static str = r#"raw string"#;
        const EXPECTED: &[Span] = &[
            Span {
                start: LineColumn { line: 1, column: 1 },
                end: LineColumn { line: 1, column: 3 },
            },
            Span {
                start: LineColumn { line: 1, column: 5 },
                end: LineColumn {
                    line: 1,
                    column: 10,
                },
            },
        ];

        try_from_test_body(CONTENT, EXPECTED);
    }
}
