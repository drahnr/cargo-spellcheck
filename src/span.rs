//! Span annotation independent compatible with proc_macro2

use crate::Range;

use super::TrimmedLiteral;
pub use proc_macro2::LineColumn;

use std::hash::{Hash, Hasher};

use anyhow::{anyhow, Error, Result};

use std::convert::TryFrom;

/// Relative span in relation
/// to the beginning of a doc comment.
///
/// Line values are 1-indexed relative, lines are inclusive.
/// Column values in UTF-8 characters in a line, 0-indexed and inclusive.
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
    /// Converts a span to a range, where `self` is converted to a range reltive to the
    /// passed span `scope`.
    /// Only works for literals spanning a single line and the scope full contains
    /// `self, otherwise an an `Err(..)` is returned.
    pub fn relative_to<X: Into<Span>>(&self, scope: X) -> Result<Range> {
        let scope: Span = scope.into();
        let scope: Range = scope.try_into()?;
        let me: Range = self.try_into()?;
        if scope.start > me.start {
            return Err(anyhow!("start of {:?} is not inside of {:?}", me, scope));
        }
        if scope.end < me.end {
            return Err(anyhow!("end of {:?} is not inside of {:?}", me, scope));
        }
        let offset = me.start - scope.start;
        let length = me.end - me.start;
        let range = Range {
            start: offset,
            end: offset + length,
        };
        Ok(range)
    }

    /// Check if `self` span covers provided `line` number, which is 1-indexed.
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

impl TryInto<Range> for Span {
    type Error = Error;
    fn try_into(self) -> Result<Range> {
        (&self).try_into()
    }
}

impl TryInto<Range> for &Span {
    type Error = Error;
    fn try_into(self) -> Result<Range> {
        if self.start.line == self.end.line {
            Ok(Range {
                start: self.start.column,
                end: self.end.column + 1,
            })
        } else {
            Err(anyhow!(
                "Start and end are not in the same line {} vs {}",
                self.start.line,
                self.end.line
            ))
        }
    }
}

impl TryFrom<(usize, Range)> for Span {
    type Error = Error;
    fn try_from(original: (usize, Range)) -> Result<Self> {
        if original.1.start < original.1.end {
            Ok(Self {
                start: LineColumn {
                    line: original.0,
                    column: original.1.start,
                },
                end: LineColumn {
                    line: original.0,
                    column: original.1.end - 1,
                },
            })
        } else {
            Err(anyhow!(
                "range must be valid to be converted to a Span {}..{}",
                original.1.start,
                original.1.end
            ))
        }
    }
}

impl From<&TrimmedLiteral> for Span {
    fn from(literal: &TrimmedLiteral) -> Self {
        literal.span()
    }
}

// impl From<(usize, Range)> for Span {
//     fn from(original: (usize, Range)) -> Self {
//         Self::try_from(original).unwrap()
//     }
// }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back() {
        const TEXT: &'static str = "Ey you!! Yes.., you there!";
        let span = Span {
            start: LineColumn {
                line: 0usize,
                column: 3usize,
            },
            end: LineColumn {
                line: 0usize,
                column: 7usize,
            },
        };

        let range = ((&span).try_into() as Result<Range>).unwrap();
        assert_eq!(range, 3..8);
        assert_eq!(&TEXT[range], "you!!");
        assert_eq!(span, (0usize, 3..8).try_into().unwrap());
    }

    // use crate::fluff_up;

    // #[test]
    // fn forth() {

    //     const CONTENT: &'static str = fluff_up!(["Omega"]);
    //     let range = ((&span).try_into() as Result<Range>).unwrap();
    //     assert_eq!(range, 3..8);
    //     assert_eq!(&TEXT[range], "you!!");
    //     assert_eq!(span, (0usize, 3..8).try_into().unwrap());
    // }
}
