//! Chunk definition for what is going to be processed by the checkers

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
}

impl ContentOrigin {
    pub fn as_path(&self) -> &Path {
        match self {
            Self::CommonMarkFile(path) => path.as_path(),
            Self::RustDocTest(path, _) => path.as_path(),
            Self::RustSourceFile(path) => path.as_path(),
        }
    }
}

/// A chunk of documentation that is supposed to be checked
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CheckableChunk {
    /// Rendered contents of a literal set or just content of a markdown file
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
        let Range { start, end } = range;
        let mut active = false;
        self.source_mapping
            .iter()
            .inspect(|x| {
                trace!(">>> item {:?}", x.0);
            })
            .skip_while(|(sub, span)| {
                sub.end <= range.start
            })
            .take_while(|(sub, span)| {
                 range.end <= sub.end
            })
            .filter(|(sub, _)| {
                // could possibly happen on empty documentation lines with `///`
                sub.len() > 0
            })
            .filter_map(|(sub, span)| {
                if (span.start.line == span.end.line)
                {
                    assert!(span.start.column <= span.end.column);
                    if span.start.column > span.end.column {
                        return None;
                    }
                }

                // assured by filter
                assert!(end > 0);

                if sub.contains(&start) {
                    if sub.contains(&(end - 1)) {
                        // complete start to end
                        active = false;
                        Some(start..end)
                    } else {
                        // only start, continue taking until end
                        active = true;
                        Some(start..sub.end)
                    }
                } else if active {
                    if sub.contains(&(end - 1)) {
                        active = false;
                        Some(sub.start..end)
                    } else {
                        Some(sub.clone())
                    }
                } else {
                    None
                }
                .map(|fragment_range| {
                    // @todo handle multiline here
                    // @todo requires knowledge of how many items are remaining in the line
                    // @todo which needs to be extracted from chunk
                    assert_eq!(span.start.line, span.end.line);
                    let mut span = span.clone();
                    // both deltas must be positive
                    // |<--------range----------->|
                    // |<-d1->|<-fragment->|<-d2->|
                    let d1 = dbg!(fragment_range.start).checked_sub(range.start).expect("d1 must be positive");
                    let d2 = range.end.checked_sub(fragment_range.end).expect("d2 must be positive");
                    // @todo count line wraps
                    span.start.column += d1 + 1; // @todo fuck me why again a magic offset!?
                    span.end.column -= d2;
                    assert!(span.start.column <= span.end.column);

                    dbg!((fragment_range, span))
                })
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
        let first = chunk.source_mapping.iter().next().unwrap().1; // @todo
        let last = chunk.source_mapping.iter().rev().next().unwrap().1; // @todo
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
        // generate  `///<space>...`
        const SOURCE: &'static str = fluff_up!(["xyz"]);
        let set = gen_literal_set(SOURCE);
        let chunk = dbg!(CheckableChunk::from_literalset(set));

        // range in `chunk.as_str()`
        const CHUNK_RANGE: Range = 1..4;

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
        let mut rdr = SOURCE.as_bytes();
        assert_eq!(
            load_span_from(&mut rdr, dbg!(*span)).expect("Span extraction must work"),
            "xyz".to_owned()
        );
        assert_eq!(span, &EXPECTED_SPAN);
    }

    #[test]
    fn find_spans_long() {
        const SOURCE: &'static str = fluff_up!(["xyz", "second", "third", "fourth"]);
        let set = gen_literal_set(SOURCE);
        let chunk = dbg!(CheckableChunk::from_literalset(set));
        const INPUT_RANGE: Range = 0..4;
        const EXPECTED_SPAN: Span = Span {
            start: LineColumn { line: 1, column: 4 },
            end: LineColumn { line: 1, column: 6 },
        };

        let res = chunk.find_spans(INPUT_RANGE.clone());
        // test deals only with a single line, so we know it only is a single entry
        assert_eq!(res.len(), 1);
        let (range, span) = dbg!(res.iter().next().unwrap());
        assert!(INPUT_RANGE.contains(&(range.start)));
        assert!(INPUT_RANGE.contains(&(range.end - 1)));
        assert_eq!(span, &EXPECTED_SPAN);
        let mut rdr = SOURCE.as_bytes();
        assert_eq!(
            load_span_from(&mut rdr, *span).expect("Span extraction must work"),
            "xyz".to_owned()
        );
    }
}
