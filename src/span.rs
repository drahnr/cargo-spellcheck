//! Span annotation independent of proc_macro2

pub use proc_macro2::LineColumn;

use std::hash::{Hash, Hasher};

/// Relative span in relation
/// to the beginning of a doc comment.
#[derive(Clone, Debug, Copy, PartialEq, Eq)]
// TODO ,Eq,PartialEq,PartialOrd,Ord
pub struct RelativeSpan {
    pub start: LineColumn,
    pub end: LineColumn,
}

impl Hash for RelativeSpan {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.start.line.hash(state);
        self.start.column.hash(state);
        self.end.line.hash(state);
        self.end.column.hash(state);
    }
}

// Span in relation to a full Document
#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: LineColumn,
    pub end: LineColumn,
}

impl Hash for Span {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.start.line.hash(state);
        self.start.column.hash(state);
        self.end.line.hash(state);
        self.end.column.hash(state);
    }
}

impl Span {
    /// Only works for literals spanning a single line.
    pub fn relative_to<X: Into<Span>>(&self, scope: X) -> anyhow::Result<Range> {
        let scope: Span = scope.into();
        let scope: Range = scope.try_into()?;
        let me: Range = self.try_into()?;
        if scope.start > me.start {
            return Err(anyhow::anyhow!(
                "start of {:?} is not inside of {:?}",
                me,
                scope
            ));
        }
        if scope.end < me.end {
            return Err(anyhow::anyhow!(
                "end of {:?} is not inside of {:?}",
                me,
                scope
            ));
        }
        let offset = me.start - scope.start;
        let length = me.end - me.start;
        let range = Range {
            start: offset,
            end: offset + length,
        };
        Ok(range)
    }

    pub fn covers_line(&self, line: usize) -> bool {
        self.end.line <= line && line >= self.start.line
    }
}

use std::convert::{From, TryInto};

impl From<proc_macro2::Span> for Span {
    fn from(original: proc_macro2::Span) -> Self {
        Self {
            start: original.start(),
            end: original.end(),
        }
    }
}

use crate::Range;

impl TryInto<Range> for Span {
    type Error = anyhow::Error;
    fn try_into(self) -> Result<Range, Self::Error> {
        if self.start.line == self.end.line {
            Ok(Range {
                start: self.start.column,
                end: self.end.column,
            })
        } else {
            Err(anyhow::anyhow!(
                "Start and end are not in the same line {} vs {}",
                self.start.line,
                self.end.line
            ))
        }
    }
}

impl TryInto<Range> for &Span {
    type Error = anyhow::Error;
    fn try_into(self) -> Result<Range, Self::Error> {
        if self.start.line == self.end.line {
            Ok(Range {
                start: self.start.column,
                end: self.end.column,
            })
        } else {
            Err(anyhow::anyhow!(
                "Start and end are not in the same line {} vs {}",
                self.start.line,
                self.end.line
            ))
        }
    }
}


impl From<(usize, Range)> for Span {
    fn from(original: (usize, Range)) -> Self {
        Self {
            start: LineColumn {
                line: original.0,
                column: original.1.start,
            },
            end: LineColumn {
                line: original.0,
                column: original.1.end,
            },
        }
    }
}