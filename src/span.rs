//! Span annotation independent of proc_macro2

pub use proc_macro2::LineColumn;

/// Relative span in relation
/// to the beginning of a doc comment.
#[derive(Clone, Debug, Copy)]
// TODO ,Eq,PartialEq,PartialOrd,Ord
pub struct RelativeSpan {
    pub start: LineColumn,
    pub end: LineColumn,
}

// Span in relation to a full Document
#[derive(Clone, Debug, Copy)]
pub struct Span {
    pub start: LineColumn,
    pub end: LineColumn,
}
