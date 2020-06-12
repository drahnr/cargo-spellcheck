use crate::span::Span;
use crate::suggestion::Suggestion;
use anyhow::{anyhow, Error, Result};
use log::trace;
use std::convert::TryFrom;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BandAid {
    /// a span, where the first line has index 1, columns are base 1 too
    pub span: Span,
    /// replacement text for the given span
    pub replacement: String,
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
            let mut span = suggestion.span.clone();
            // @todo this is a hack and should be documented better
            // @todo not sure why the offset of two is necessary
            // @todo but it works consistently
            let doc_comment_to_file_offset = 2;
            span.start.column += doc_comment_to_file_offset;
            span.end.column += doc_comment_to_file_offset;
            Ok(Self {
                span,
                replacement: replacement.to_owned(),
            })
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
