//! A mistake bandaid.
//!
//! A `BandAid` covers the mistake with a suggested replacement, as picked by
//! the user.

use crate::documentation::Span;

#[cfg(test)]
pub(crate) mod tests;

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
