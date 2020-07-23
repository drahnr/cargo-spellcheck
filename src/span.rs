//! `Span` annotation, independent yet compatible with `proc_macro2::Span`
//!
//! Re-uses `LineColumn`, where `.line` is 1-indexed, and `.column`s are 0-indexed,
//! `.end` is inclusive.

use crate::Range;

use super::TrimmedLiteral;
pub use proc_macro2::LineColumn;

use std::hash::{Hash, Hasher};

use anyhow::{bail, Error, Result};

use std::convert::TryFrom;

use super::CheckableChunk;

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
            bail!("start of {:?} is not inside of {:?}", me, scope)
        }
        if scope.end < me.end {
            bail!("end of {:?} is not inside of {:?}", me, scope)
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

    /// extract a `Range` which maps to `self` as
    /// `span` maps to `range`, where `range` is relative to `full_content`
    fn extract_sub_range_from_span(
        &self,
        span: Span,
        range: Range,
        full_content: &str,
    ) -> Result<Range> {
        let s = &full_content[range.clone()];
        let offset = range.start;
        // relative to the range given / offset
        let mut start = 0usize;
        let mut end = 0usize;
        let state = span.start;
        for (idx, _c, line, col) in s.chars().enumerate().scan(state, |state, (idx, c)| {
            let x = (idx, c, state.line, state.column);
            match c {
                '\n' => {
                    state.line += 1;
                    state.column = 0;
                }
                _ => state.column += 1,
            }
            Some(x)
        }) {
            if line < self.start.line {
                continue;
            }
            if line > self.end.line {
                bail!("Moved beyond anticipated line")
            }

            if line >= self.end.line && col > self.end.column {
                bail!("Moved beyond anticipated column and last line")
            }
            if line == self.start.line && col == self.start.column {
                start = idx;
                // do not continue, the first line/column could be the last one too!
            }
            end = idx;
            // if the iterations go to the end of the string, the condition will never be met inside the loop
            if line == self.end.line && col == self.end.column {
                break;
            }

            if line > self.end.line {
                bail!("Moved beyond anticipated line")
            }

            if line >= self.end.line && col > self.end.column {
                bail!("Moved beyond anticipated column and last line")
            }
        }

        let range2 = (offset + start)..(offset + end + 1);
        assert!(range2.len() <= range.len());
        if span.start.line == span.end.line {
            assert_eq!(range.len(), span.end.column - span.start.column + 1);
        }
        return Ok(range2);
    }

    /// Convert a given span `self` into a `Range`
    ///
    /// The `Chunk` has a associated `Span` (or a set of `Range` -> `Span` mappings)
    /// which are used to map.
    pub fn to_content_range(&self, chunk: &CheckableChunk) -> Result<Range> {
        if chunk.fragment_count() == 0 {
            bail!("Chunk contains 0 fragments")
        }
        for (fragment_range, fragment_span) in chunk
            .iter()
            // pre-filter to reduce too many calls to `extract_sub_range`
            .filter(|(fragment_range, fragment_span)| {
                log::trace!(
                    "extracting sub from {:?} ::: {:?} -> {:?}",
                    self,
                    &fragment_range,
                    &fragment_span
                );
                self.start.line >= fragment_span.start.line
                    && self.end.line <= fragment_span.end.line
            })
        {
            match self.extract_sub_range_from_span(
                *fragment_span,
                fragment_range.clone(),
                chunk.as_str(),
            ) {
                Ok(range2) => return Ok(range2),
                Err(_e) => continue,
            }
        }
        bail!("The chunk internal map from range to span did not contain an overlapping entry")
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
            bail!(
                "Start and end are not in the same line {} vs {}",
                self.start.line,
                self.end.line
            )
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
            bail!(
                "range must be valid to be converted to a Span {}..{}",
                original.1.start,
                original.1.end
            )
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
    use crate::action::bandaid::tests::load_span_from;
    use crate::documentation::literalset::tests::gen_literal_set;
    use crate::{chyrp_up, fluff_up};
    use crate::{LineColumn, Range, Span};

    #[test]
    fn span_to_range_singleline() {
        let _ = env_logger::builder().is_test(true).try_init();

        const CONTENT: &'static str = fluff_up!("Itsyou!!", "", "Game-Over!!", "");
        let set = gen_literal_set(CONTENT);
        let chunk = dbg!(CheckableChunk::from_literalset(set));

        // assuming a `///<space>` comment
        const TRIPPLE_SLASH_PLUS_SPACE: usize = 4;

        // within a file
        const INPUTS: &[Span] = &[
            Span {
                start: LineColumn {
                    line: 1usize,
                    column: 3usize + TRIPPLE_SLASH_PLUS_SPACE,
                },
                end: LineColumn {
                    line: 1usize,
                    column: 7usize + TRIPPLE_SLASH_PLUS_SPACE,
                },
            },
            Span {
                start: LineColumn {
                    line: 3usize,
                    column: 0usize + TRIPPLE_SLASH_PLUS_SPACE,
                },
                end: LineColumn {
                    line: 3usize,
                    column: 8usize + TRIPPLE_SLASH_PLUS_SPACE,
                },
            },
        ];

        // ranges to be used with `chunk.as_str()`
        // remember that ///<space> counts towards the range!
        // and that newlines are also one char
        const EXPECTED_RANGE: &[Range] = &[4..9, 13..22];

        // note that this may only be single lines, since `///` implies separate literals
        // and as such multiple spans
        const FRAGMENT_STR: &[&'static str] = &["you!!", "Game-Over"];

        for (input, expected, fragment) in itertools::cons_tuples(
            INPUTS
                .iter()
                .zip(EXPECTED_RANGE.iter())
                .zip(FRAGMENT_STR.iter()),
        ) {
            log::trace!(
                ">>>>>>>>>>>>>>>>\ninput: {:?}\nexpected: {:?}\nfragment:>{}<",
                input,
                expected,
                fragment
            );
            let range = input
                .to_content_range(&chunk)
                .expect("Inputs are sane, conversion must work.");
            assert_eq!(range, *expected);
            // make sure the span covers what we expect it to cover
            assert_eq!(
                load_span_from(CONTENT.as_bytes(), input.clone()).unwrap(),
                fragment.to_owned()
            );
            assert_eq!(&(&chunk.as_str()[range]), fragment);
        }
    }

    #[test]
    fn span_to_range_multiline() {
        let _ = env_logger::builder().is_test(true).try_init();

        const CONTENT: &'static str = chyrp_up!("Ey you!! Yes.., you there!", "", "GameChange", "");
        let set = gen_literal_set(dbg!(CONTENT));
        let chunk = dbg!(CheckableChunk::from_literalset(set));

        // assuming a `#[doc=r#"` comment
        const HASH_BRACKET_DOC_EQ_RAW_HASH_QUOTE: usize = 9;
        // const QUOTE_HASH: usize = 2;

        // within a file
        const INPUTS: &[Span] = &[
            // full
            Span {
                start: LineColumn {
                    line: 1usize,
                    column: 0usize + HASH_BRACKET_DOC_EQ_RAW_HASH_QUOTE,
                },
                end: LineColumn {
                    line: 3usize,
                    column: 10usize,
                },
            },
            // sub
            Span {
                start: LineColumn {
                    line: 1usize,
                    column: 3usize + HASH_BRACKET_DOC_EQ_RAW_HASH_QUOTE,
                },
                end: LineColumn {
                    line: 1usize,
                    column: 7usize + HASH_BRACKET_DOC_EQ_RAW_HASH_QUOTE,
                },
            },
            Span {
                start: LineColumn {
                    line: 3usize,
                    column: 0usize,
                },
                end: LineColumn {
                    line: 3usize,
                    column: 9usize,
                },
            },
        ];

        const EXPECTED_RANGE: &[Range] = &[0..(26 + 1 + 0 + 1 + 10 + 1), 3..8, 28..38];

        // note that this may only be single lines, since `///` implies separate literals
        // and as such multiple spans
        const FRAGMENT_STR: &[&'static str] = &[
            r#"Ey you!! Yes.., you there!

GameChange
"#,
            "you!!",
            "GameChange",
        ];

        for (input, expected, fragment) in itertools::cons_tuples(
            INPUTS
                .iter()
                .zip(EXPECTED_RANGE.iter())
                .zip(FRAGMENT_STR.iter()),
        ) {
            log::trace!(
                ">>>>>>>>>>>>>>>>\ninput: {:?}\nexpected: {:?}\nfragment:>{}<",
                input,
                expected,
                fragment
            );

            let range = dbg!(input)
                .to_content_range(&chunk)
                .expect("Inputs are sane, conversion must work. qed");
            assert_eq!(range, *expected);

            assert_eq!(
                load_span_from(CONTENT.as_bytes(), input.clone()).unwrap(),
                fragment.to_owned()
            );
            assert_eq!(&(&chunk.as_str()[range]), fragment);
        }
    }
}
