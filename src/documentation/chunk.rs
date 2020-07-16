//! Chunk definition for what is going to be processed by the checkers
//!
//! A chunk consists of multiple consecutive literals joined by newlines.

use super::*;

use indexmap::IndexMap;
use std::path::Path;

use crate::documentation::PlainOverlay;
use crate::{Range, Span};
/// Definition of the source of a checkable chunk
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum ContentOrigin {
    CommonMarkFile(PathBuf),
    RustDocTest(PathBuf, Span), // span is just there to disambiguiate
    RustSourceFile(PathBuf),
    #[cfg(test)]
    TestEntity,
}

impl ContentOrigin {
    pub fn as_path(&self) -> &Path {
        match self {
            Self::CommonMarkFile(path) => path.as_path(),
            Self::RustDocTest(path, _) => path.as_path(),
            Self::RustSourceFile(path) => path.as_path(),
            #[cfg(test)]
            Self::TestEntity => {
                lazy_static::lazy_static! {
                    static ref TEST_ENTITY: PathBuf = PathBuf::from("/tmp/test/entity");
                };
                TEST_ENTITY.as_path()
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
}

impl std::hash::Hash for CheckableChunk {
    fn hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        self.content.hash(hasher);
        // order is consistent
        self.source_mapping.iter().for_each(|t| {
            t.hash(hasher);
        });
    }
}

impl CheckableChunk {
    /// Specific to rust source code, either as part of doc test comments or file scope
    pub fn from_literalset(set: LiteralSet) -> Self {
        set.into_chunk()
    }

    /// Load content from string, may contain markdown content
    pub fn from_str(content: &str, source_mapping: IndexMap<Range, Span>) -> Self {
        Self::from_string(content.to_string(), source_mapping)
    }

    pub fn from_string(content: String, source_mapping: IndexMap<Range, Span>) -> Self {
        Self {
            content,
            source_mapping,
        }
    }

    /// Obtain an accessor object containing mapping and string repr, removing the markdown anotations.
    pub fn erase_markdown(&self) -> PlainOverlay {
        PlainOverlay::erase_markdown(self)
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
        trace!(
            "############################################ chunk find_span {:?}",
            &range
        );

        let Range { start, end } = range;
        let mut active = false;
        self.source_mapping
            .iter()
            .skip_while(|(fragment_range, _span)| fragment_range.end <= start)
            .take_while(|(fragment_range, _span)| end <= fragment_range.end)
            .inspect(|x| {
                trace!(">>> item {:?} ∈ {:?}", &range, x.0);
            })
            .filter(|(fragment_range, _)| {
                // could possibly happen on empty documentation lines with `///`
                fragment_range.len() > 0
            })
            .filter_map(|(fragment_range, fragment_span)| {
                // trim range so we only capture the relevant part
                let sub_fragment_range = std::cmp::max(fragment_range.start, range.start)..std::cmp::min(fragment_range.end, range.end);
                trace!(
                    ">> fragment: span:{:?} => range:{:?} | sub:{:?} -> sub_fragment{:?}",
                    &fragment_span,
                    &fragment_range,
                    range,
                    &sub_fragment_range,
                );

                if sub_fragment_range.len() == 0 {
                    return None;
                }

                // take the full fragment string, we need to count newlines before and after
                let s = &self.as_str()[fragment_range.clone()];
                // relative to the range given / offset
                let mut sub_fragment_span = fragment_span.clone();
                let state: LineColumn = fragment_span.start;
                for (idx, _c, cursor) in s.chars().enumerate().scan(state, |state, (idx, c)| {
                    let x: (usize, char, LineColumn) = (idx, c, state.clone());
                    match c {
                        '\r' => {} // @todo assert the following char is a \n
                        '\n' => {
                            state.line += 1;
                            state.column = 0;
                        }
                        _ => state.column += 1,
                    }
                    Some(x)
                }) {
                    if idx < sub_fragment_range.start {
                        continue;
                    }
                    if idx == sub_fragment_range.start {
                        sub_fragment_span.start = cursor;
                        sub_fragment_span.end = cursor; // assure this is valid
                        continue;
                    }
                    sub_fragment_span.end = cursor; // always set, even if we never reach the end of fragment
                    if idx >= (sub_fragment_range.end - 1) {
                        break;
                    }
                }

                let _ = dbg!(&sub_fragment_span);
                let _ = dbg!(&sub_fragment_range);
                if sub_fragment_span.start.line == sub_fragment_span.end.line {
                    assert_eq!(dbg!(sub_fragment_span.end.column - sub_fragment_span.start.column + 1),
                        dbg!(&sub_fragment_range).len());
                } else {
                    assert!(
                        fragment_span.start.column <= fragment_span.end.column
                    );
                }
                log::warn!(
                    ">> sub_fragment range={:?} span={:?} => {}",
                    &sub_fragment_range,
                    &sub_fragment_span,
                    self.display(sub_fragment_range.clone()),
                );

                Some((sub_fragment_range, sub_fragment_span))

            })
            .collect::<IndexMap<_, _>>()
    }

    pub fn as_str(&self) -> &str {
        self.content.as_str()
    }

    pub fn display(&self, range: Range) -> ChunkDisplay {
        ChunkDisplay::from((self, range))
    }

    pub fn iter(&self) -> indexmap::map::Iter<Range, Span> {
        self.source_mapping.iter()
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

/// Extract lines together with associated `Range`s relative to str `s`
///
/// Easily collectable into a `HashMap`.
fn lines_with_ranges<'a>(s: &'a str) -> impl Iterator<Item = (Range, &'a str)> + Clone {
    // @todo line consumes \r\n and \n so the ranges could off by 1 on windows
    // @todo requires a custom impl of `lines()` iterator
    s.lines().scan(
        0usize,
        |offset: &mut usize, line: &'_ str| -> Option<(Range, &'_ str)> {
            let n = line.chars().count();
            let range = *offset..*offset + n + 1;
            *offset += n + 1; // for newline, see @todo above
            Some((range, line))
        },
    )
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
        let _first = chunk.source_mapping.iter().next().unwrap().1; // @todo
        let _last = chunk.source_mapping.iter().rev().next().unwrap().1; // @todo
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
        let _literal = self.0;
        let start = self.1.start;
        let end = self.1.end;

        assert!(start <= end);

        // content without quote characters
        let data = self.0.as_str();

        // colour the preceding quote character
        // and the context preceding the highlight
        let ctx1 = if start < data.len() {
            context.apply_to(&data[..start])
        } else {
            oob.apply_to("!!!")
        };

        // highlight the given range
        let highlight = if end > data.len() {
            oob.apply_to(&data[start..])
        } else {
            highlight.apply_to(&data[start..end])
        };

        // color trailing context if any as well as the closing quote character
        let ctx2 = if end <= data.len() {
            context.apply_to(&data[end..])
        } else {
            oob.apply_to("!!!")
        };

        write!(formatter, "{}{}{}", ctx1, highlight, ctx2)
    }
}

#[cfg(test)]
mod test {
    use super::action::bandaid::tests::load_span_from;
    use super::literalset::tests::gen_literal_set;
    use super::*;

    #[test]
    fn find_spans_simple() {
        let _ = env_logger::builder().is_test(true).try_init();

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
        let _ = env_logger::builder().is_test(true).try_init();

        const SOURCE: &'static str = fluff_up!(["xyz", "second", "third", "fourth"]);
        let set = gen_literal_set(SOURCE);
        let chunk = dbg!(CheckableChunk::from_literalset(set));
        const CHUNK_RANGES: &[Range] =
            &[1..4, (4 + 1 + 1 + 6 + 1 + 1)..(4 + 1 + 1 + 6 + 1 + 1 + 5)];
        const EXPECTED_SPANS: &[Span] = &[
            Span {
                start: LineColumn { line: 1, column: 4 },
                end: LineColumn { line: 1, column: 6 },
            },
            Span {
                start: LineColumn { line: 3, column: 4 },
                end: LineColumn { line: 3, column: 8 },
            },
        ];
        const EXPECTED_STR: &[&'static str] = &["xyz", "third"];

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
        let _ = env_logger::builder().is_test(true).try_init();

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
}
