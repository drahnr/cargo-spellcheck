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
        let doc_comment_to_file_offset = 0;
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
    use crate::checker::{Checker, dummy::DummyChecker};
    use crate::documentation::*;
    use crate::span::Span;
    use log::debug;
    use proc_macro2::{LineColumn, Literal};
    use std::path::PathBuf;

    #[test]
    fn try_from_string_works() {
        let _ = env_logger::builder()
            .filter(None, log::LevelFilter::Debug)
            .is_test(true)
            .try_init();

        let mut d = Documentation::new();
        let dummy_path = PathBuf::from("dummy/dummy.rs");
        d.append_literal(&dummy_path, Literal::string("This has four literals"));
        let suggestion_set = DummyChecker::check(&d, &()).expect("DummyChecker failed");

        assert_eq!(suggestion_set.len(), 1);
        assert_eq!(suggestion_set.total_count(), 4);
        for (_, suggestion) in suggestion_set {
            let bandaid =
                BandAid::try_from((suggestion.iter().nth(0).unwrap(), 0)).expect("try_from failed");
            debug!("{:?}", &bandaid);
            debug!("Sug span: {:?}", suggestion.iter().nth(0).unwrap().span);
            assert_eq!(bandaid.replacement, "replacement_0");
            assert_eq!(bandaid.span, suggestion.iter().nth(0).unwrap().span);
        }
    }

    #[test]
    fn try_from_raw_string_works() {
        let mut d = Documentation::new();
        let dummy_path = PathBuf::from("dummy/dummy.rs");
        d.append_literal(&dummy_path, Literal::string(r#"Some raw string"#));
        let literal_set = DummyChecker::check(&d, &()).expect("DummyChecker failed");

        assert_eq!(literal_set.len(), 1);
        assert_eq!(literal_set.total_count(), 3);
        for (_, suggestion) in literal_set {
            let bandaid =
                BandAid::try_from((suggestion.iter().nth(0).unwrap(), 0)).expect("try_from failed");
            assert_eq!(bandaid.replacement, "literal");
            assert_eq!(bandaid.span, suggestion.iter().nth(0).unwrap().span);
        }
    }

    #[test]
    fn try_from_multiple_works() {
        let mut d = Documentation::new();
        let dummy_path = PathBuf::from("dummy/dummy.rs");
        let dummy_path2 = PathBuf::from("dummy/dummy2.rs");
        d.append_literal(&dummy_path, Literal::string("this is a literal"));
        d.append_literal(&dummy_path2, Literal::string(" another one"));

        let literal_set = DummyChecker::check(&d, &()).expect("DummyChecker failed");
        assert_eq!(literal_set.len(), 2);
        assert_eq!(literal_set.total_count(), 6);

        for (_, suggestion) in literal_set {
            let bandaid =
                BandAid::try_from((suggestion.iter().nth(0).unwrap(), 0)).expect("failed");
            trace!("{:?}", bandaid);
            assert_eq!(bandaid.replacement, "literal");
            assert_eq!(bandaid.span, suggestion.iter().next().unwrap().span);
        }
    }
}
