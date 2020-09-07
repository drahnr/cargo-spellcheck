//! Chunk definition for what is going to be processed by the checkers
//!
//! A chunk consists of multiple consecutive literals joined by newlines.

use super::*;

use indexmap::IndexMap;
use std::path::Path;

use crate::documentation::PlainOverlay;
use crate::{util::sub_chars, Range, Span};

/// Definition of the source of a checkable chunk
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum ContentOrigin {
    /// A common mark file at given path.
    CommonMarkFile(PathBuf),
    /// A rustdoc comment, part of file reference by path in span.
    RustDocTest(PathBuf, Span),
    /// Full rust source file.
    RustSourceFile(PathBuf),
    /// A test entity for a rust file, with no meaning outside of test.
    #[cfg(test)]
    TestEntityRust,
    /// A test entity for a cmark file, with no meaning outside of test.
    #[cfg(test)]
    TestEntityCommonMark,
}

impl ContentOrigin {
    /// Represent the content origin as [path](std::path::PathBuf).
    ///
    /// For unit and integration tests, two additional hardcoded variants
    /// are available, which resolve to static paths:
    /// `TestEntityRust` variant becomes `/tmp/test/entity.rs`,
    /// `TestEntityCommonMark` variant becomes `/tmp/test/entity.md`.
    pub fn as_path(&self) -> &Path {
        match self {
            Self::CommonMarkFile(path) => path.as_path(),
            Self::RustDocTest(path, _) => path.as_path(),
            Self::RustSourceFile(path) => path.as_path(),
            #[cfg(test)]
            Self::TestEntityCommonMark => {
                lazy_static::lazy_static! {
                    static ref TEST_ENTITY_CMARK: PathBuf = PathBuf::from("/tmp/test/entity.md");
                };
                TEST_ENTITY_CMARK.as_path()
            }
            #[cfg(test)]
            Self::TestEntityRust => {
                lazy_static::lazy_static! {
                    static ref TEST_ENTITY_RUST: PathBuf = PathBuf::from("/tmp/test/entity.rs");
                };
                TEST_ENTITY_RUST.as_path()
            }
        }
    }
}

impl fmt::Display for ContentOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.as_path().display())
    }
}

/// A chunk of documentation that is supposed to be checked
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CheckableChunk {
    /// Rendered contents of a literal set or just content of a markdown file, e.g. a comment of two lines is
    /// represented as ' First Line\n second line' in `rendered` where the whitespaces are preserved.
    content: String,
    /// Mapping from range within `content` and
    /// `Span` referencing the location within the source file.
    /// For a markdown file i.e. this would become a single entry spanning from start to end.
    source_mapping: IndexMap<Range, Span>,
    /// Track what kind of comment the chunk is
    variant: CommentVariant,
}

impl std::hash::Hash for CheckableChunk {
    fn hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        self.content.hash(hasher);
        // order is consistent
        self.source_mapping.iter().for_each(|t| {
            t.hash(hasher);
        });
        self.variant.hash(hasher);
    }
}

impl CheckableChunk {
    /// Specific to rust source code, either as part of doc test comments or file scope.
    pub fn from_literalset(set: LiteralSet) -> Self {
        set.into_chunk()
    }

    /// Load content from string, may contain common mark content.
    pub fn from_str(
        content: &str,
        source_mapping: IndexMap<Range, Span>,
        variant: CommentVariant,
    ) -> Self {
        Self::from_string(content.to_string(), source_mapping, variant)
    }

    /// Load content from string, may contain common mark content.
    pub fn from_string(
        content: String,
        source_mapping: IndexMap<Range, Span>,
        variant: CommentVariant,
    ) -> Self {
        Self {
            content,
            source_mapping,
            variant,
        }
    }

    /// Find which part of the range maps to which span.
    /// Note that Range can very well be split into multiple fragments
    /// where each of them can be mapped to a potentially non-continuous
    /// span.
    ///
    /// Example:
    ///
    /// ```text,ignore
    /// 0..40 -> [
    ///           (0,10) => (1,0)->(3,5),
    ///           (10,12) => (3,6)->(3,7),
    ///           (13,17) => (4,0)->(4,3),
    /// ]
    /// ```
    pub(super) fn find_spans(&self, range: Range) -> IndexMap<Range, Span> {
        trace!(target: "find_spans",
            "############################################ chunk find_span {:?}",
            &range
        );

        let Range { start, end } = range;
        self.source_mapping
            .iter()
            .skip_while(|(fragment_range, _span)| fragment_range.end <= start)
            .take_while(|(fragment_range, _span)| fragment_range.start < end)
            .inspect(|x| {
                trace!(target: "find_spans", ">>> item {:?} ∈ {:?}", &range, x.0);
            })
            .filter(|(fragment_range, _)| {
                // could possibly happen on empty documentation lines with `///`
                fragment_range.len() > 0
            })
            .filter_map(|(fragment_range, fragment_span)| {
                // trim range so we only capture the relevant part
                let sub_fragment_range = std::cmp::max(fragment_range.start, range.start)
                    ..std::cmp::min(fragment_range.end, range.end);

                trace!(target: "find_spans",
                    ">> fragment: span: {:?} => range: {:?} | sub: {:?} -> sub_fragment: {:?}",
                    &fragment_span,
                    &fragment_range,
                    range,
                    &sub_fragment_range,
                );

                log::trace!(target: "find_spans",
                    "[f]display;\n>{}<",
                    ChunkDisplay::try_from((self, fragment_range.clone()))
                        .expect("must be convertable")
                );
                log::trace!(target: "find_spans",
                    "[f]content;\n>{}<",
                    crate::util::sub_chars(self.as_str(), fragment_range.clone())
                );

                if sub_fragment_range.len() == 0 {
                    log::trace!(target: "find_spans","sub fragment is zero, dropping!");
                    return None;
                }

                if let Some(span_len) = fragment_span.one_line_len() {
                    debug_assert_eq!(span_len, fragment_range.len());
                }
                // take the full fragment string, we need to count newlines before and after
                let s = sub_chars(self.as_str(), fragment_range.clone());
                // relative to the range given / offset
                let shift = sub_fragment_range.start - fragment_range.start;
                let mut sub_fragment_span = fragment_span.clone();
                let state: LineColumn = fragment_span.start;
                for (idx, c, cursor) in s.chars().enumerate().scan(state, |state, (idx, c)| {
                    let x: (usize, char, LineColumn) = (idx, c, state.clone());
                    match c {
                        '\n' => {
                            state.line += 1;
                            state.column = 0;
                        }
                        _ => state.column += 1,
                    }
                    Some(x)
                }) {
                    trace!(target: "find_spans", "char[{}]: {}", idx, c);
                    if idx == shift {
                        sub_fragment_span.start = cursor;
                    }
                    sub_fragment_span.end = cursor; // always set, even if we never reach the end of fragment
                    if idx >= (sub_fragment_range.len() + shift - 1) {
                        break;
                    }
                }

                if let Some(sub_fragment_span_len) = sub_fragment_span.one_line_len() {
                    debug_assert_eq!(sub_fragment_span_len, sub_fragment_range.len());
                }
                log::trace!(
                    ">> sub_fragment range={:?} span={:?} => {}",
                    &sub_fragment_range,
                    &sub_fragment_span,
                    self.display(sub_fragment_range.clone()),
                );

                Some((sub_fragment_range, sub_fragment_span))
            })
            .collect::<IndexMap<_, _>>()
    }

    /// Extract all spans which at least partially overlap with range,
    /// i.e. report all spans that either
    ///  - contain `range.start`
    ///  - contain `range.end`
    ///  - are totally enclosed in `range`
    ///
    /// Example:
    ///
    /// Below setup results in `[s2, s3, s4]`
    ///
    /// ```text,ignore
    /// |-- s1 --|-- s2 --|-- s3 --|-- s4 --|
    ///             |----- range -----|
    /// ```
    pub fn find_covered_spans<'a>(&'a self, range: Range) -> impl Iterator<Item = &'a Span> {
        let Range { start, end } = range;
        self.source_mapping
            .iter()
            .skip_while(move |(fragment_range, _)| fragment_range.end <= start)
            .take_while(move |(fragment_range, _)| fragment_range.start <= end)
            .filter_map(|(fragment_range, fragment_span)| {
                // could possibly happen on empty documentation lines with `///`
                if fragment_range.is_empty() {
                    Some(fragment_span)
                } else {
                    None
                }
            })
    }

    /// Yields a set of ranges covering all spanned lines (the full line)
    pub fn find_covered_lines<'i>(&'i self, range: Range) -> Vec<Range> {
        // assumes the _mistake_ is within one line
        // if not we chop it down to the first line
        let mut acc = Vec::with_capacity(32);
        let mut iter = self.as_str().chars().enumerate();

        let mut last_newline_idx = 0usize;
        // simulate the previous newline was at virtual `-1`
        let mut state_idx = 0usize;
        let mut state_c = '\n';
        loop {
            if let Some((idx, c)) = iter.next() {
                if c == '\n' {
                    if range.start <= idx {
                        // do not include the newline
                        acc.push(last_newline_idx..idx);
                    }
                    last_newline_idx = idx + 1;
                    if last_newline_idx >= range.end {
                        break;
                    }
                }
                state_c = c;
                state_idx = idx;
            } else {
                // if the previous character was a new line,
                // such that the common mark chunk ended with
                // a newline, we do not want to append another empty line
                // for no reason, we include empty lines for `\n\n` though
                if state_c != '\n' {
                    // we want to include the last character
                    acc.push(last_newline_idx..(state_idx + 1));
                }
                break;
            };
        }
        acc
    }

    /// Obtain the content as `str` representation.
    pub fn as_str(&self) -> &str {
        self.content.as_str()
    }

    /// Get the display wrapper type to be used with i.e. `format!(..)`.
    pub fn display(&self, range: Range) -> ChunkDisplay {
        ChunkDisplay::from((self, range))
    }

    /// Iterate over all ranges and the associated span.
    pub fn iter(&self) -> indexmap::map::Iter<Range, Span> {
        self.source_mapping.iter()
    }

    /// Number of fragments.
    ///
    /// A fragment is a continuous sub-string which is not
    /// split up any further.
    pub fn fragment_count(&self) -> usize {
        self.source_mapping.len()
    }

    /// Obtain an accessor object containing mapping and string repr, removing the markdown anotations.
    pub fn erase_cmark(&self) -> PlainOverlay {
        PlainOverlay::erase_cmark(self)
    }

    /// Obtain the length in characters.
    pub fn len_in_chars(&self) -> usize {
        self.content.chars().count()
    }

    /// The variant type of comment.
    pub fn variant(&self) -> CommentVariant {
        self.variant
    }
}

/// Convert the clusters of one file into a source description as well
/// as well as vector of checkable chunks.
impl From<Clusters> for Vec<CheckableChunk> {
    fn from(clusters: Clusters) -> Vec<CheckableChunk> {
        clusters
            .set
            .into_iter()
            .map(|literal_set| CheckableChunk::from_literalset(literal_set))
            .collect::<Vec<_>>()
    }
}

use std::fmt;

/// A display style wrapper for a trimmed literal.
///
/// Allows better display of coverage results without code duplication.
///
/// Consists of literal reference and a relative range to the start of the literal.
#[derive(Debug, Clone)]
pub struct ChunkDisplay<'a>(pub &'a CheckableChunk, pub Range);

impl<'a, R> From<(R, Range)> for ChunkDisplay<'a>
where
    R: Into<&'a CheckableChunk>,
{
    fn from(tuple: (R, Range)) -> Self {
        let tuple0 = tuple.0.into();
        Self(tuple0, tuple.1)
    }
}

use anyhow::{Error, Result};
use std::convert::TryFrom;

impl<'a, R> TryFrom<(R, Span)> for ChunkDisplay<'a>
where
    R: Into<&'a CheckableChunk>,
{
    type Error = Error;
    fn try_from(tuple: (R, Span)) -> Result<Self> {
        let chunk = tuple.0.into();
        let span = tuple.1;
        let range = span.to_content_range(chunk)?;
        Ok(Self(chunk, range))
    }
}

impl<'a> Into<(&'a CheckableChunk, Range)> for ChunkDisplay<'a> {
    fn into(self) -> (&'a CheckableChunk, Range) {
        (self.0, self.1)
    }
}

impl<'a> fmt::Display for ChunkDisplay<'a> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use console::Style;

        // the contextual characters not covered by range `self.1`
        let context = Style::new().on_black().bold().cyan();
        // highlight the mistake
        let highlight = Style::new().on_black().bold().underlined().red().italic();
        // a special style for any errors, to visualize out of bounds access
        let oob = Style::new().blink().bold().on_yellow().red();

        // simplify
        let literal = self.0;
        let start = self.1.start;
        let end = self.1.end;

        assert!(start <= end);

        // content without quote characters
        let data = literal.as_str();

        // colour the preceding quote character
        // and the context preceding the highlight
        let s = sub_chars(data, 0..start);
        let ctx1 = if start < literal.len_in_chars() {
            context.apply_to(s.as_str())
        } else {
            oob.apply_to("!!!")
        };

        // highlight the given range
        let s = sub_chars(data, start..end);
        let highlight = if end > literal.len_in_chars() {
            oob.apply_to(s.as_str())
        } else {
            highlight.apply_to(s.as_str())
        };

        // color trailing context if any as well as the closing quote character
        let s = sub_chars(data, end..literal.len_in_chars());
        let ctx2 = if end <= literal.len_in_chars() {
            context.apply_to(s.as_str())
        } else {
            oob.apply_to("!!!")
        };

        write!(formatter, "{}{}{}", ctx1, highlight, ctx2)
    }
}

#[cfg(test)]
mod test {
    use super::literalset::tests::gen_literal_set;
    use super::util::load_span_from;
    use super::*;

    #[test]
    fn find_spans_emoji() {
        const TEST: &str = r##"ab **🐡** xy"##;

        let chunk = CheckableChunk::from_str(
            TEST,
            indexmap::indexmap! { 0..11 => Span {
                start: LineColumn {
                    line: 1usize,
                    column: 4usize,
                },
                end: LineColumn {
                    line: 1usize,
                    column: 14usize,
                },
            }},
            CommentVariant::CommonMark
        );

        assert_eq!(chunk.find_spans(0..2).len(), 1);
        assert_eq!(chunk.find_spans(5..6).len(), 1);
        assert_eq!(chunk.find_spans(9..11).len(), 1);
        assert_eq!(chunk.find_spans(9..20).len(), 1);
    }

    #[test]
    fn find_spans_simple() {
        let _ = env_logger::builder()
            .is_test(true)
            .filter(None, log::LevelFilter::Trace)
            .try_init();

        // generate  `///<space>...`
        const SOURCE: &'static str = fluff_up!(["xyz"]);
        let set = gen_literal_set(SOURCE);
        let chunk = dbg!(CheckableChunk::from_literalset(set));

        // range in `chunk.as_str()`
        // " xyz"
        const CHUNK_RANGE: Range = 1..4;

        // "/// xyz"
        //  0123456
        const EXPECTED_SPAN: Span = Span {
            start: LineColumn { line: 1, column: 4 },
            end: LineColumn { line: 1, column: 6 },
        };

        let range2span = chunk.find_spans(CHUNK_RANGE.clone());
        // test deals only with a single line, so we know it only is a single entry
        assert_eq!(range2span.len(), 1);

        // assure the range is correct given the chunk
        assert_eq!("xyz", &chunk.as_str()[CHUNK_RANGE.clone()]);

        let (range, span) = dbg!(range2span.iter().next().unwrap());
        assert!(CHUNK_RANGE.contains(&(range.start)));
        assert!(CHUNK_RANGE.contains(&(range.end - 1)));
        assert_eq!(
            load_span_from(SOURCE.as_bytes(), dbg!(*span)).expect("Span extraction must work"),
            "xyz".to_owned()
        );
        assert_eq!(span, &EXPECTED_SPAN);
    }

    #[test]
    fn find_spans_multiline() {
        let _ = env_logger::builder()
            .is_test(true)
            .filter(None, log::LevelFilter::Trace)
            .try_init();

        const SOURCE: &'static str = fluff_up!(["xyz", "second", "third", "Converts a span to a range, where `self` is converted to a range reltive to the",
             "passed span `scope`."] @ "       "
        );
        let set = gen_literal_set(SOURCE);
        let chunk = dbg!(CheckableChunk::from_literalset(set));
        const SPACES: usize = 7;
        const TRIPLE_SLASH_SPACE: usize = 4;
        const CHUNK_RANGES: &[Range] =
            &[1..4, (4 + 1 + 1 + 6 + 1 + 1)..(4 + 1 + 1 + 6 + 1 + 1 + 5)];
        const EXPECTED_SPANS: &[Span] = &[
            Span {
                start: LineColumn {
                    line: 1,
                    column: SPACES + TRIPLE_SLASH_SPACE + 0,
                },
                end: LineColumn {
                    line: 1,
                    column: SPACES + TRIPLE_SLASH_SPACE + 2,
                },
            },
            Span {
                start: LineColumn {
                    line: 3,
                    column: SPACES + TRIPLE_SLASH_SPACE + 0,
                },
                end: LineColumn {
                    line: 3,
                    column: SPACES + TRIPLE_SLASH_SPACE + 4,
                },
            },
            Span {
                start: LineColumn {
                    line: 4,
                    column: SPACES + TRIPLE_SLASH_SPACE + 0,
                },
                end: LineColumn {
                    line: 4,
                    column: SPACES + TRIPLE_SLASH_SPACE + 78,
                },
            },
            Span {
                start: LineColumn {
                    line: 5,
                    column: SPACES + TRIPLE_SLASH_SPACE + 0,
                },
                end: LineColumn {
                    line: 5,
                    column: SPACES + TRIPLE_SLASH_SPACE + 19,
                },
            },
        ];
        const EXPECTED_STR: &[&'static str] = &[
            "xyz",
            "third",
            "Converts a span to a range, where `self` is converted to a range reltive to the",
            "passed span `scope`.",
        ];

        for (query_range, expected_span, expected_str) in itertools::cons_tuples(
            CHUNK_RANGES
                .iter()
                .zip(EXPECTED_SPANS.iter())
                .zip(EXPECTED_STR.iter()),
        ) {
            let range2span = chunk.find_spans(query_range.clone());
            // test deals only with a single line, so we know it only is a single entry
            assert_eq!(range2span.len(), 1);
            let (range, span) = dbg!(range2span.iter().next().unwrap());
            assert!(query_range.contains(&(range.start)));
            assert!(query_range.contains(&(range.end - 1)));
            assert_eq!(
                load_span_from(SOURCE.as_bytes(), *span).expect("Span extraction must work"),
                expected_str.to_owned()
            );
            assert_eq!(span, expected_span);
        }
    }

    #[test]
    fn find_spans_chyrp() {
        let _ = env_logger::builder()
            .is_test(true)
            .filter(None, log::LevelFilter::Trace)
            .try_init();

        const SOURCE: &'static str = chyrp_up!(["Amsel", "Wacholderdrossel", "Buchfink"]);
        let set = gen_literal_set(SOURCE);
        let chunk = dbg!(CheckableChunk::from_literalset(set));

        const CHUNK_RANGES: &[Range] = &[0..(5 + 1 + 16 + 1 + 8)];
        const EXPECTED_SPANS: &[Span] = &[Span {
            start: LineColumn {
                line: 1,
                column: 0 + 9,
            }, // prefix is #[doc=r#"
            end: LineColumn { line: 3, column: 7 }, // suffix is pointeless
        }];

        assert_eq!(
            dbg!(&EXPECTED_SPANS[0]
                .to_content_range(&chunk)
                .expect("Must be ok to extract span from chunk")),
            dbg!(&CHUNK_RANGES[0])
        );

        const EXPECTED_STR: &[&'static str] = &[r#"Amsel
Wacholderdrossel
Buchfink"#];

        assert_eq!(EXPECTED_STR[0], chunk.as_str());

        for (query_range, expected_span, expected_str) in itertools::cons_tuples(
            CHUNK_RANGES
                .iter()
                .zip(EXPECTED_SPANS.iter())
                .zip(EXPECTED_STR.iter()),
        ) {
            let range2span = chunk.find_spans(query_range.clone());
            // test deals only with a single line, so we know it only is a single entry
            assert_eq!(range2span.len(), 1);
            let (range, span) = dbg!(range2span.iter().next().unwrap());
            assert!(query_range.contains(&(range.start)));
            assert!(query_range.contains(&(range.end - 1)));
            assert_eq!(
                load_span_from(SOURCE.as_bytes(), *span).expect("Span extraction must work"),
                expected_str.to_owned()
            );
            assert_eq!(span, expected_span);
        }
    }

    #[test]
    fn find_spans_inclusive() {
        let _ = env_logger::builder().is_test(true).try_init();

        const SOURCE: &'static str = fluff_up!(["Some random words"]);
        let set = gen_literal_set(SOURCE);
        let chunk = dbg!(CheckableChunk::from_literalset(set));
        // a range inside the span
        const CHUNK_RANGE: Range = 4..15;

        const EXPECTED_SPAN: Span = Span {
            start: LineColumn { line: 1, column: 3 },
            end: LineColumn {
                line: 1,
                column: 20,
            },
        };

        let mut range2span = chunk.find_covered_spans(CHUNK_RANGE.clone());

        // assure the range is correct given the chunk
        assert_eq!("e random wo", &chunk.as_str()[CHUNK_RANGE.clone()]);

        let span = dbg!(range2span.next().unwrap());
        assert_eq!(
            load_span_from(SOURCE.as_bytes(), dbg!(*span)).expect("Span extraction must work"),
            " Some random words".to_owned()
        );
        assert_eq!(span, &EXPECTED_SPAN);
        // test deals only with a single line, so we know it only is a single entry
        assert_eq!(range2span.count(), 0);
    }

    #[test]
    fn find_spans_inclusive_multiline() {
        let _ = env_logger::builder().is_test(true).try_init();

        const SOURCE: &'static str = fluff_up!(["xyz", "second", "third", "Converts a span to a range, where `self` is converted to a range reltive to the",
             "passed span `scope`."] @ "       "
        );
        let set = gen_literal_set(SOURCE);
        let chunk = dbg!(CheckableChunk::from_literalset(set));
        const SPACES: usize = 7;
        const TRIPLE_SLASH_SPACE: usize = 3;
        const CHUNK_RANGE: Range = 7..22;
        const EXPECTED_SPANS: &[Span] = &[
            Span {
                start: LineColumn {
                    line: 2,
                    column: SPACES + TRIPLE_SLASH_SPACE + 0,
                },
                end: LineColumn {
                    line: 2,
                    column: SPACES + TRIPLE_SLASH_SPACE + 6,
                },
            },
            Span {
                start: LineColumn {
                    line: 3,
                    column: SPACES + TRIPLE_SLASH_SPACE + 0,
                },
                end: LineColumn {
                    line: 3,
                    column: SPACES + TRIPLE_SLASH_SPACE + 5,
                },
            },
            Span {
                start: LineColumn {
                    line: 4,
                    column: SPACES + TRIPLE_SLASH_SPACE + 0,
                },
                end: LineColumn {
                    line: 4,
                    column: SPACES + TRIPLE_SLASH_SPACE + 79,
                },
            },
        ];

        let range2span = chunk.find_covered_spans(CHUNK_RANGE);

        for (spans, expected) in range2span.zip(EXPECTED_SPANS) {
            assert_eq!(spans, expected);
        }
    }
}
