//! Chunk definition for what is going to be processed by the checkers
//!
//! A chunk consists of multiple consecutive literals joined by newlines.

use super::*;

use indexmap::IndexMap;
use std::convert::TryFrom;
use std::fmt;
use std::path::Path;

use crate::{
    util::{sub_char_range, sub_chars},
    Range, Span,
};
use crate::{Ignores, PlainOverlay};

/// Definition of the source of a checkable chunk
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum ContentOrigin {
    /// A `Cargo.toml` manifest that contains a `description` field.
    CargoManifestDescription(PathBuf),
    /// A common mark file at given path.
    CommonMarkFile(PathBuf),
    /// A rustdoc comment, part of file reference by path in span.
    RustDocTest(PathBuf, Span),
    /// Full rust source file.
    RustSourceFile(PathBuf),
    /// A test entity for a rust file, with no meaning outside of test.
    TestEntityRust,
    /// A test entity for a cmark file, with no meaning outside of test.
    TestEntityCommonMark,
}

impl ContentOrigin {
    /// Represent the content origin as [path](std::path::PathBuf).
    ///
    /// For unit and integration tests, two additional hardcoded variants are
    /// available, which resolve to static paths: `TestEntityRust` variant
    /// becomes `/tmp/test/entity.rs`, `TestEntityCommonMark` variant becomes
    /// `/tmp/test/entity.md`.
    pub fn as_path(&self) -> &Path {
        match self {
            Self::CargoManifestDescription(path) => path.as_path(),
            Self::CommonMarkFile(path) => path.as_path(),
            Self::RustDocTest(path, _) => path.as_path(),
            Self::RustSourceFile(path) => path.as_path(),
            Self::TestEntityCommonMark => {
                lazy_static::lazy_static! {
                    static ref TEST_ENTITY_CMARK: PathBuf = PathBuf::from("/tmp/test/entity.md");
                };
                TEST_ENTITY_CMARK.as_path()
            }
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

/// A chunk of documentation that is supposed to be checked.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CheckableChunk {
    /// Rendered contents of a literal set or just content of a markdown file,
    /// e.g. a comment of two lines is represented as ' First Line\n second
    /// line' in `rendered` where the whitespaces are preserved.
    content: String,
    /// Mapping from range within `content` and `Span` referencing the location
    /// within the source file. For a markdown file i.e. this would become a
    /// single entry spanning from start to end.
    source_mapping: IndexMap<Range, Span>,
    /// Track what kind of comment the chunk is.
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
    /// Specific to rust source code, either as part of doc test comments or
    /// file scope.
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

    /// Find which part of the range maps to which span. Note that Range can
    /// very well be split into multiple fragments where each of them can be
    /// mapped to a potentially non-continuous span.
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
    pub fn find_spans(&self, range: Range) -> IndexMap<Range, Span> {
        log::trace!(target: "find_spans", "Chunk find_span {range:?}");

        let Range { start, end } = range;
        self.source_mapping
            .iter()
            .skip_while(|(fragment_range, _span)| fragment_range.end <= start)
            .take_while(|(fragment_range, _span)| fragment_range.start < end)
            .inspect(|x| {
                log::trace!(target: "find_spans", ">>> item {:?} ∈ {:?}", range, x.0);
            })
            .filter(|(fragment_range, _)| {
                // could possibly happen on empty documentation lines with `///`
                !fragment_range.is_empty()
            })
            .map(|(fragment_range, fragment_span)| {
                // trim range so we only capture the relevant part
                let sub_fragment_range = std::cmp::max(fragment_range.start, range.start)
                    ..std::cmp::min(fragment_range.end, range.end);
                (fragment_span, fragment_range, sub_fragment_range)
            })
            .inspect(|(fragment_span, fragment_range, sub_fragment_range)| {
                let (fragment_span, fragment_range, sub_fragment_range) =
                    (fragment_span, fragment_range, sub_fragment_range.clone());
                log::trace!(target: "find_spans",
                    ">> fragment: span: {fragment_span:?} => range: {fragment_range:?} | sub: {range:?} -> sub_fragment: {sub_fragment_range:?}",
                );

                log::trace!(target: "find_spans",
                    "[f]display;\n>{}<",
                    ChunkDisplay::from((self, *fragment_range))
                );
                log::trace!(target: "find_spans",
                    "[f]content;\n>{}<",
                    sub_chars(self.as_str(), (*fragment_range).clone())
                );
            })
            .filter_map(|(fragment_span, fragment_range, sub_fragment_range)| {
                if sub_fragment_range.is_empty() {
                    log::trace!(target: "find_spans","sub fragment is zero, dropping!");
                    return None;
                }
                if let Some(span_len) = fragment_span.one_line_len() {
                    debug_assert_eq!(span_len, fragment_range.len());
                }
                Some((fragment_span, fragment_range, sub_fragment_range))
            })
            .filter_map(|(fragment_span, fragment_range, sub_fragment_range)| {
                // take the full fragment string, we need to count newlines before and after
                let s = sub_char_range(self.as_str(), fragment_range.clone());

                // relative to the range given / offset
                let shift = sub_fragment_range.start - fragment_range.start;
                // state
                let mut sub_fragment_span = *fragment_span;
                let mut cursor: LineColumn = fragment_span.start;
                let mut iter = s.chars().enumerate().peekable();
                let mut started = true;
                'w: while let Some((idx, c)) = iter.next() {
                    if idx == shift {
                        sub_fragment_span.start = cursor;
                        started = true;
                    }
                    if idx >= (sub_fragment_range.len() + shift - 1) {
                        sub_fragment_span.end = cursor;
                        break 'w;
                    }
                    if iter.peek().is_none() && started {
                        sub_fragment_span.end = cursor;
                    }
                    // FIXME what about \n\r or \r\n or \r ?
                    match c {
                        '\n' => {
                            cursor.line += 1;
                            cursor.column = 0;
                        }
                        _ => cursor.column += 1,
                    }
                }

                if let Some(sub_fragment_span_len) = sub_fragment_span.one_line_len() {
                    debug_assert_eq!(sub_fragment_span_len, sub_fragment_range.len());
                }
                log::trace!(
                    ">> sub_fragment range={sub_fragment_range:?} span={sub_fragment_span:?} => {}",
                    self.display(sub_fragment_range.clone()),
                );

                Some((sub_fragment_range, sub_fragment_span))
            })
            .collect::<IndexMap<_, _>>()
    }

    /// Extract all spans which at least partially overlap with range, i.e.
    /// report all spans that either
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
    ///
    /// Attention:
    ///
    /// For large `#[doc="long multiline text"]` comments, the covered span
    /// might be large (i.e. just one single) which leads to a surprising result
    /// of just one span for a relatively small input `range`.
    ///
    /// Below setup results in `[s0]`
    ///
    /// ```text,ignore
    /// |---...--- s0 ----------------------...---|
    ///             |--- range ---|
    /// ```
    ///
    pub fn find_covered_spans(&self, range: Range) -> impl Iterator<Item = &'_ Span> {
        let Range { start, end } = range;
        self.source_mapping
            .iter()
            .skip_while(move |(fragment_range, _)| fragment_range.end <= start)
            .take_while(move |(fragment_range, _)| fragment_range.start <= end)
            .filter_map(|(fragment_range, fragment_span)| {
                // could possibly happen on empty documentation lines with `///`
                // TODO: is_empty() throws disambiguity error
                if fragment_range.is_empty() {
                    None
                } else {
                    Some(fragment_span)
                }
            })
    }

    /// Yields a set of ranges covering all spanned lines (the full line).
    pub fn find_covered_lines(&self, range: Range) -> Vec<Range> {
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

    /// Extract the overall length of all covered lines as they appear in the
    /// origin.
    pub fn extract_line_lengths(&self) -> Result<Vec<usize>> {
        let line_ranges = self.find_covered_lines(0..self.len_in_chars());
        let lengths = line_ranges
            .iter()
            .try_fold(Vec::new(), |mut acc, line_range| {
                let spans = self.find_spans(line_range.clone());
                if let Some(span) = spans.get(line_range) {
                    acc.push(span.start.column + line_range.len());
                    Ok(acc)
                } else if let Some(span) = self.source_mapping.get(line_range) {
                    // if the span was not found, it should still be in the whole source mapping
                    acc.push(span.start.column + line_range.len());
                    Ok(acc)
                } else {
                    Err(Error::InvalidLineRange {
                        line_range: line_range.clone(),
                        source_mapping: self.source_mapping.clone(),
                    })
                }
            })?;

        Ok(lengths)
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
    /// A fragment is a continuous sub-string which is not split up any further.
    pub fn fragment_count(&self) -> usize {
        self.source_mapping.len()
    }

    /// Obtain an accessor object containing mapping and string representation,
    /// removing the markdown annotations.
    pub fn erase_cmark(&self, ignores: &Ignores) -> PlainOverlay {
        PlainOverlay::erase_cmark(self, ignores)
    }

    /// Obtain the length in characters.
    pub fn len_in_chars(&self) -> usize {
        self.content.chars().count()
    }

    /// The variant type of comment.
    pub fn variant(&self) -> CommentVariant {
        self.variant.clone()
    }
}

/// Convert the clusters of one file into a source description as well as well
/// as vector of checkable chunks.
impl From<Clusters> for Vec<CheckableChunk> {
    fn from(clusters: Clusters) -> Vec<CheckableChunk> {
        clusters
            .set
            .into_iter()
            .map(CheckableChunk::from_literalset)
            .collect::<Vec<_>>()
    }
}

/// A display style wrapper for a trimmed literal.
///
/// Allows better display of coverage results without code duplication.
///
/// Consists of literal reference and a relative range to the start of the
/// literal.
#[derive(Debug, Clone)]
pub struct ChunkDisplay<'a>(pub &'a CheckableChunk, pub Range);

impl<'a, C> From<(C, &Range)> for ChunkDisplay<'a>
where
    C: Into<&'a CheckableChunk>,
{
    fn from(tuple: (C, &Range)) -> Self {
        let tuple0 = tuple.0.into();
        Self(tuple0, tuple.1.clone())
    }
}

impl<'a, C> From<(C, Range)> for ChunkDisplay<'a>
where
    C: Into<&'a CheckableChunk>,
{
    fn from(tuple: (C, Range)) -> Self {
        let tuple0 = tuple.0.into();
        Self(tuple0, tuple.1)
    }
}

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

impl<'a> From<ChunkDisplay<'a>> for (&'a CheckableChunk, Range) {
    fn from(val: ChunkDisplay<'a>) -> Self {
        (val.0, val.1)
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

        write!(formatter, "{ctx1}{highlight}{ctx2}")
    }
}
