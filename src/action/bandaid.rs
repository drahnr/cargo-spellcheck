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
    use crate::checker::tests::TestChecker;
    use crate::checker::Checker;
    use crate::documentation::*;
    use proc_macro2::{LineColumn, Literal};
    use std::path::PathBuf;

    #[test]
    fn try_from_string_works() {
        let mut d = Documentation::new();
        let dummy_path = PathBuf::from("dummy/dummy.rs");
        d.append_literal(&dummy_path, Literal::string("literal"));
        let c = TestChecker::check(&d, &()).expect("TestChecker failed");

        assert_eq!(c.len(), 1);
        for (_, suggestion) in c {
            let bandaid =
                BandAid::try_from((suggestion.iter().nth(0).unwrap(), 0)).expect("try_from failed");
            assert_eq!(bandaid.replacement, "literal");
        }
    }

    #[test]
    fn try_from_raw_string_works() {
        let mut d = Documentation::new();
        let dummy_path = PathBuf::from("dummy/dummy.rs");
        d.append_literal(&dummy_path, Literal::string(r#"literal"#));
        let c = TestChecker::check(&d, &()).expect("TestChecker failed");

        assert_eq!(c.len(), 1);
        for (_, suggestion) in c {
            let bandaid =
                BandAid::try_from((suggestion.iter().nth(0).unwrap(), 0)).expect("try_from failed");
            assert_eq!(bandaid.replacement, "literal");
            assert_eq!(
                bandaid.span,
                crate::span::Span {
                    start: LineColumn { line: 1, column: 3 },
                    end: LineColumn { line: 1, column: 9 },
                }
            );
        }
    }

    #[test]
    fn try_from_multiple_works() {
        let mut d = Documentation::new();
        let dummy_path = PathBuf::from("dummy/dummy.rs");
        let dummy_path2 = PathBuf::from("dummy/dummy2.rs");
        d.append_literal(&dummy_path, Literal::string("this is a literal"));
        d.append_literal(&dummy_path2, Literal::string("another one"));

        let c = TestChecker::check(&d, &()).expect("TestChecker failed");
        assert_eq!(c.len(), 2);

        let mut count = 0;
        for (_, suggestion) in c {
            let bandaid =
                BandAid::try_from((suggestion.iter().nth(0).unwrap(), 0)).expect("failed");
            trace!("{:?}", bandaid);
            assert_eq!(bandaid.replacement, "literal");
            assert_eq!(
                bandaid.span,
                crate::span::Span {
                    start: LineColumn { line: 1, column: 3 },
                    end: LineColumn {
                        line: 1,
                        column: count * 3 + 6
                    },
                }
            );
            count += 1;
        }
    }
}
