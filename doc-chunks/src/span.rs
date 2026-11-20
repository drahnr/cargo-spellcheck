//! `Span` annotation, independent yet compatible with `proc_macro2::Span`
//!
//! Re-uses `LineColumn`, where `.line` is 1-indexed, and `.column`s are
//! 0-indexed, `.end` is inclusive.

use super::TrimmedLiteral;
use crate::util;
use crate::Range;
pub use proc_macro2::LineColumn;

use std::hash::{Hash, Hasher};

use crate::errors::*;

use std::convert::TryFrom;

use super::CheckableChunk;

#[cfg(test)]
mod tests;

/// Relative span in relation to the beginning of a doc comment.
///
/// Line values are 1-indexed relative, lines are _inclusive_. Column values in
/// UTF-8 characters in a line, 0-indexed and _inclusive_.
#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub struct Span {
    /// Start of the span, inclusive, see
    /// [`LineColumn`](proc_macro2::LineColumn).
    pub start: LineColumn,
    /// End of the span, inclusive, see [`LineColumn`](proc_macro2::LineColumn).
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
    /// Converts a span to a range, where `self` is converted to a range
    /// relative to the passed span `scope`. Only works for literals spanning a
    /// single line and the scope full contains `self, otherwise an an `Err(..)`
    /// is returned.
    pub fn relative_to<X: Into<Span>>(&self, scope: X) -> Result<Range> {
        let scope: Span = scope.into();
        let scope: Range = scope.try_into()?;
        let me: Range = self.try_into()?;
        if scope.start > me.start {
            return Err(Error::Span(format!(
                "start of {me:?} is not inside of {scope:?}",
            )));
        }
        if scope.end < me.end {
            return Err(Error::Span(format!(
                "end of {me:?} is not inside of {scope:?}",
            )));
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

    /// If this one resembles a single line, returns the a `Some(len)` value.
    /// For multilines this cannot account for the length.
    pub fn one_line_len(&self) -> Option<usize> {
        if self.start.line == self.end.line {
            Some(self.end.column + 1 - self.start.column)
        } else {
            None
        }
    }

    ///  Check if `self` covers multiple lines
    pub fn is_multiline(&self) -> bool {
        self.start.line != self.end.line
    }

    /// Convert a given span `self` into a `Range`
    ///
    /// The `Chunk` has a associated `Span` (or a set of `Range` -> `Span`
    /// mappings) which are used to map.
    pub fn to_content_range(&self, chunk: &CheckableChunk) -> Result<Range> {
        if chunk.fragment_count() == 0 {
            return Err(Error::Span("Chunk contains 0 fragments".to_string()));
        }
        for (fragment_range, fragment_span) in chunk
            .iter()
            // pre-filter to reduce too many calls to `extract_sub_range`
            .filter(|(fragment_range, fragment_span)| {
                log::trace!(
                    "extracting sub from {self:?} ::: {fragment_range:?} -> {fragment_span:?}",
                );
                !fragment_range.is_empty()
                    && fragment_span.start.line <= self.start.line
                    && self.end.line <= fragment_span.end.line
            })
        {
            match extract_sub_range_from_span(
                chunk.as_str(),
                *fragment_span,
                fragment_range.clone(),
                *self,
            ) {
                Ok(fragment_sub_range) => return Ok(fragment_sub_range),
                Err(_e) => continue,
            }
        }
        Err(Error::Span(
            "The chunk internal map from range to span did not contain an overlapping entry"
                .to_string(),
        ))
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
            Err(Error::Span(format!(
                "Start and end are not in the same line {} vs {}",
                self.start.line, self.end.line
            )))
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
            Err(Error::Span(format!(
                "range must be valid to be converted to a Span {}..{}",
                original.1.start, original.1.end
            )))
        }
    }
}

impl TryFrom<(usize, std::ops::RangeInclusive<usize>)> for Span {
    type Error = Error;
    fn try_from(original: (usize, std::ops::RangeInclusive<usize>)) -> Result<Self> {
        if original.1.start() <= original.1.end() {
            Ok(Self {
                start: LineColumn {
                    line: original.0,
                    column: *original.1.start(),
                },
                end: LineColumn {
                    line: original.0,
                    column: *original.1.end(),
                },
            })
        } else {
            Err(Error::Span(format!(
                "range must be valid to be converted to a Span {}..{}",
                original.1.start(),
                original.1.end()
            )))
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

/// extract a `Range` which maps to `self` as `span` maps to `range`, where
/// `range` is relative to `full_content`
fn extract_sub_range_from_span(
    full_content: &str,
    span: Span,
    range: Range,
    sub_span: Span,
) -> Result<Range> {
    if (sub_span.start.line, sub_span.start.column) < (span.start.line, span.start.column)
        || (sub_span.end.line, sub_span.end.column) > (span.end.line, span.end.column)
    {
        return Err(Error::Span(format!(
            "sub span {sub_span:?} is not contained in {span:?}",
        )));
    }

    if let Some(span_len) = span.one_line_len() {
        debug_assert_eq!(range.len(), span_len);
    }

    // extract the fragment of interest to which both `range` and `span` correspond.
    let s = util::sub_chars(full_content, range.clone());
    let offset = range.start;
    // relative to the range given / offset
    let mut start = 0usize;
    let mut end = 0usize;
    for (_c, _byte_offset, idx, LineColumn { line, column }) in
        util::iter_with_line_column_from(s.as_str(), span.start)
    {
        if line < sub_span.start.line {
            continue;
        }
        if line > sub_span.end.line {
            return Err(Error::Span(format!(
                "range must be valid to be converted to a Span {}..{}",
                range.start, range.end
            )));
        }

        if line == sub_span.start.line && column < sub_span.start.column {
            continue;
        }

        if line >= sub_span.end.line && column > sub_span.end.column {
            return Err(Error::Span(
                "Moved beyond anticipated column and last line".to_string(),
            ));
        }
        if line == sub_span.start.line && column == sub_span.start.column {
            start = idx;
            // do not continue, the first line/column could be the last one too!
        }
        end = idx;
        // if the iterations go to the end of the string, the condition will never be met inside the loop
        if line == sub_span.end.line && column == sub_span.end.column {
            break;
        }

        if line > sub_span.end.line {
            return Err(Error::Span("Moved beyond anticipated line".to_string()));
        }

        if line >= sub_span.end.line && column > sub_span.end.column {
            return Err(Error::Span(
                "Moved beyond anticipated column and last line".to_string(),
            ));
        }
    }

    let sub_range = (offset + start)..(offset + end + 1);
    assert!(sub_range.len() <= range.len());

    if let Some(sub_span_len) = sub_span.one_line_len() {
        debug_assert_eq!(sub_range.len(), sub_span_len);
    }

    Ok(sub_range)
}
