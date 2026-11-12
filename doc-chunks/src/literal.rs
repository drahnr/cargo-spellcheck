use crate::errors::*;
use crate::util::{self, sub_chars};
use crate::{Range, Span};

use proc_macro2::LineColumn;

use std::fmt;

/// Broad source category used when grouping comment literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentVariantCategory {
    /// Comment variant will end up in documentation.
    Doc,
    /// Comment variant is only visible in source code.
    Dev,
    /// It's a common mark file, and we actually don't know.
    CommonMark,
    /// Toml entries and such.
    Unmergable,
}

/// Source syntax and rendering delimiters for a comment literal.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
#[non_exhaustive]
pub enum CommentVariant {
    /// `///`
    TripleSlash,
    /// `//!`
    DoubleSlashEM,
    /// `/*!`
    SlashAsteriskEM,
    /// `/**`
    SlashAsteriskAsterisk,
    /// `/*`
    SlashAsterisk,
    /// `#[doc=` with actual prefix like `#[doc=` and the total length of `r###`
    /// etc. including `r` but without `"`
    MacroDocEqStr(String, usize),
    /// Legacy marker for `#[doc = macro!(...)]`, retained for compatibility.
    /// Extraction now tracks unexpanded macros as fragments and rejects selections containing them.
    MacroDocEqMacro,
    /// Commonmark File
    CommonMark,
    /// Developer line comment
    DoubleSlash,
    /// Developer block comment
    SlashStar,
    /// Unknown Variant
    Unknown,
    /// Toml entry
    TomlEntry,
}

impl Default for CommentVariant {
    fn default() -> Self {
        CommentVariant::Unknown
    }
}

impl CommentVariant {
    /// Obtain the comment variant category.
    pub fn category(&self) -> CommentVariantCategory {
        match self {
            Self::TripleSlash => CommentVariantCategory::Doc,
            Self::DoubleSlashEM => CommentVariantCategory::Doc,
            Self::MacroDocEqStr(_, _) => CommentVariantCategory::Doc,
            Self::MacroDocEqMacro => CommentVariantCategory::Doc,
            Self::SlashAsteriskEM => CommentVariantCategory::Doc,
            Self::SlashAsteriskAsterisk => CommentVariantCategory::Doc,
            Self::CommonMark => CommentVariantCategory::CommonMark,
            Self::TomlEntry => CommentVariantCategory::Unmergable,
            _ => CommentVariantCategory::Dev,
        }
    }
    /// Return the prefix string.
    ///
    /// Does not include whitespaces for `///` and `//!` variants!
    pub fn prefix_string(&self) -> String {
        match self {
            CommentVariant::TripleSlash => "///".into(),
            CommentVariant::DoubleSlashEM => "//!".into(),
            CommentVariant::MacroDocEqMacro => "".into(),
            CommentVariant::MacroDocEqStr(d, p) => {
                let raw = match p {
                    // TODO: make repeated doc-attribute prefixes configurable at the reflow layer.
                    0 => "\"".to_owned(),
                    x => format!("r{}\"", "#".repeat(x.saturating_sub(1))),
                };
                format!(r#"{d}{raw}"#)
            }
            CommentVariant::CommonMark => "".to_string(),
            CommentVariant::DoubleSlash => "//".to_string(),
            CommentVariant::SlashStar => "/*".to_string(),
            CommentVariant::SlashAsterisk => "/*".to_string(),
            CommentVariant::SlashAsteriskEM => "/*!".to_string(),
            CommentVariant::SlashAsteriskAsterisk => "/**".to_string(),
            CommentVariant::TomlEntry => "".to_owned(),
            unhandled => {
                unreachable!("String representation for comment variant {unhandled:?} exists. qed")
            }
        }
    }
    /// Return length (in bytes) of comment prefix for each variant.
    ///
    /// By definition matches the length of `prefix_string`.
    pub fn prefix_len(&self) -> usize {
        match self {
            CommentVariant::TripleSlash | CommentVariant::DoubleSlashEM => 3,
            CommentVariant::MacroDocEqMacro => 0,
            CommentVariant::MacroDocEqStr(d, p) => d.len() + *p + 1,
            CommentVariant::SlashAsterisk => 2,
            CommentVariant::SlashAsteriskEM | CommentVariant::SlashAsteriskAsterisk => 3,
            _ => self.prefix_string().len(),
        }
    }

    /// Return suffix of different comment variants
    pub fn suffix_len(&self) -> usize {
        match self {
            CommentVariant::MacroDocEqStr(_, 0) => 2,
            CommentVariant::MacroDocEqStr(_, p) => p + 1,
            CommentVariant::SlashAsteriskAsterisk
            | CommentVariant::SlashAsteriskEM
            | CommentVariant::SlashAsterisk => 2,
            CommentVariant::MacroDocEqMacro => 0,
            _ => 0,
        }
    }

    /// Return string which will be appended to each line
    pub fn suffix_string(&self) -> String {
        match self {
            CommentVariant::MacroDocEqStr(_, p) if *p == 0 || *p == 1 => r#""]"#.to_string(),
            CommentVariant::MacroDocEqStr(_, p) => {
                r#"""#.to_string() + &"#".repeat(p.saturating_sub(1)) + "]"
            }
            CommentVariant::SlashAsteriskAsterisk
            | CommentVariant::SlashAsteriskEM
            | CommentVariant::SlashAsterisk => "*/".to_string(),
            _ => "".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum LineDelimiter {
    Lf,
    CrLf,
}

impl LineDelimiter {
    fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }
}

/// Concrete comment text with syntax delimiters and a source span for its trimmed content.
/// Trimming removes delimiters, not prose whitespace; this type does not expand macros.
#[derive(Clone)]
pub struct TrimmedLiteral {
    /// Source syntax used to render this literal.
    variant: CommentVariant,
    /// Original source span of the content, excluding prefix and suffix delimiters.
    span: Span,
    /// Original literal text, including prefix and suffix delimiters.
    rendered: String,
    /// ASCII prefix length in bytes and characters.
    pre: usize,
    /// ASCII suffix length in bytes and characters.
    post: usize,
    /// Length of rendered **minus** `pre` and `post` in UTF-8 characters.
    len_in_chars: usize,

    /// Source newline style, independent of trimmed or synthesized content.
    preferred_line_delimiter: Option<LineDelimiter>,
}

impl std::cmp::PartialEq for TrimmedLiteral {
    fn eq(&self, other: &Self) -> bool {
        if self.rendered != other.rendered {
            return false;
        }
        if self.pre != other.pre {
            return false;
        }
        if self.post != other.post {
            return false;
        }
        if self.len() != other.len() {
            return false;
        }
        if self.span != other.span {
            return false;
        }
        if self.variant != other.variant {
            return false;
        }
        if self.preferred_line_delimiter != other.preferred_line_delimiter {
            return false;
        }

        true
    }
}

impl std::cmp::Eq for TrimmedLiteral {}

impl std::hash::Hash for TrimmedLiteral {
    fn hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        self.variant.hash(hasher);
        self.rendered.hash(hasher);
        self.span.hash(hasher);
        self.pre.hash(hasher);
        self.post.hash(hasher);

        self.len_in_chars.hash(hasher);
        self.preferred_line_delimiter.hash(hasher);
    }
}

/// Return a source line without its terminator and its preferred newline style.
/// An unterminated final line inherits the nearest preceding physical terminator.
fn source_line_with_delimiter(content: &str, line: usize) -> Option<(&str, Option<LineDelimiter>)> {
    let mut previous_delimiter = None;
    for (index, source_line) in content.split_inclusive('\n').enumerate() {
        let (text, delimiter) = if let Some(text) = source_line.strip_suffix("\r\n") {
            (text, Some(LineDelimiter::CrLf))
        } else if let Some(text) = source_line.strip_suffix('\n') {
            (text, Some(LineDelimiter::Lf))
        } else {
            (source_line, None)
        };
        if index + 1 == line {
            return Some((text, delimiter.or(previous_delimiter)));
        }
        previous_delimiter = delimiter.or(previous_delimiter);
    }
    None
}

/// Adjust the provided span by a number of `pre` and `post` characters.
fn trim_span(content: &str, span: &mut Span, pre: usize, post: usize) {
    span.start.column += pre;
    if span.end.column >= post {
        span.end.column -= post;
    } else {
        // look for the last character in the previous line
        let previous_line_length = content
            .chars()
            .rev()
            // assumes \n, we want to skip the first one from the back
            .skip(post + 1)
            .take_while(|c| *c != '\n')
            .count();
        span.end = LineColumn {
            line: span.end.line - 1,
            column: previous_line_length,
        };
    }
}

/// Detect the comment variant based on the span based str content.
///
/// Became necessary, since the `proc_macro2::Span` does not distinguish between
/// `#[doc=".."]` and `/// ..` comment variants, and for one, and the span can't
/// cover both correctly.
fn detect_comment_variant(
    content: &str,
    rendered: &String,
    mut span: Span,
) -> Result<(CommentVariant, Span, usize, usize)> {
    let prefix_span = Span {
        start: crate::LineColumn {
            line: span.start.line,
            column: 0,
        },
        end: crate::LineColumn {
            line: span.start.line,
            column: span.start.column.saturating_sub(1),
        },
    };
    let prefix = util::load_span_from(content.as_bytes(), prefix_span)?
        .trim_start()
        .to_string();

    let (variant, span, pre, post) = if rendered.starts_with("///") || rendered.starts_with("//!") {
        let pre = 3; // `///`
        let post = 0; // trailing `\n` is already accounted for above

        span.start.column += pre;

        // must always be a single line
        assert_eq!(span.start.line, span.end.line);
        // Desugared doc attributes escape quotes that were literal comment text.
        // Reading the original source preserves both spelling and column offsets.

        let variant = if rendered.starts_with("///") {
            CommentVariant::TripleSlash
        } else {
            CommentVariant::DoubleSlashEM
        };

        (variant, span, pre, post)
    } else if rendered.starts_with("/*") && rendered.ends_with("*/") {
        let variant = if rendered.starts_with("/*!") {
            CommentVariant::SlashAsteriskEM
        } else if rendered.starts_with("/**") {
            CommentVariant::SlashAsteriskAsterisk
        } else {
            CommentVariant::SlashAsterisk
        };

        let pre = variant.prefix_len();
        let post = variant.suffix_len();

        #[cfg(debug_assertions)]
        let orig = span;

        trim_span(rendered, &mut span, pre, post);

        #[cfg(debug_assertions)]
        {
            let raw = util::load_span_from(&mut content.as_bytes(), orig)?;
            // Empty content has a reversed inclusive span, which cannot be read.
            let adjusted_len = if rendered.len() == pre + post {
                0
            } else {
                util::load_span_from(&mut content.as_bytes(), span)?.len()
            };

            // we know pre and post only consist of single byte characters
            // so `.len()` is way faster here yet correct.
            assert_eq!(adjusted_len + pre + post, raw.len());
        }

        (variant, span, pre, post)
    } else {
        // Delimiter lengths apply to the original literal text; adjust its content span below.

        // The parser has already validated the literal. Determine its ASCII
        // delimiters without interpreting escapes or changing source offsets.
        let (pre, post) = if rendered.starts_with('"') && rendered.ends_with('"') {
            (1, 1)
        } else if let Some(raw) = rendered.strip_prefix('r') {
            let hashes = raw.bytes().take_while(|byte| *byte == b'#').count();
            let pre = hashes + 2;
            let post = hashes + 1;
            if rendered.as_bytes().get(pre - 1) != Some(&b'"')
                || !rendered.ends_with(&format!("\"{}", "#".repeat(hashes)))
            {
                return Err(Error::Span(format!(
                    "Invalid raw documentation string: {}",
                    rendered
                )));
            }
            (pre, post)
        } else {
            return Err(Error::Span(format!(
                "Expected a documentation string: {}",
                rendered
            )));
        };

        let length = rendered.chars().count().saturating_sub(pre + post);
        let mut positions = util::iter_with_line_column_from(rendered, span.start)
            .skip(pre)
            .take(length)
            .map(|(_, _, _, cursor)| cursor);
        if let Some(start) = positions.next() {
            span.start = start;
            // The last content character may be a newline on the preceding
            // line, not column zero of the closing delimiter's line.
            span.end = positions.last().unwrap_or(start);
        } else {
            span.start.column += pre;
            span.end.column = span.start.column - 1;
        }

        (
            CommentVariant::MacroDocEqStr(prefix, pre.saturating_sub(1)),
            span,
            pre,
            post,
        )
    };
    Ok((variant, span, pre, post))
}

impl TrimmedLiteral {
    pub(crate) fn load_from(content: &str, mut span: Span) -> Result<Self> {
        // Token rendering can escape comment text and break its source mapping.
        // Read the source span directly, then identify and trim its syntax delimiters.

        // proc_macro2 ends are exclusive; our source spans are inclusive.
        span.end.column = span.end.column.saturating_sub(1);

        let mut rendered = util::load_span_from(content.as_bytes(), span)?;
        let source_line = source_line_with_delimiter(content, span.start.line);
        let preferred_line_delimiter = source_line.and_then(|(_, delimiter)| delimiter);
        if rendered.starts_with("///") || rendered.starts_with("//!") {
            // Synthetic comment spans can include a CR. Read the local line
            // without its LF/CRLF terminator, including at an unterminated EOF.
            let (line, _) = source_line.ok_or_else(|| {
                Error::Span("Documentation comment starts outside the source".to_owned())
            })?;
            rendered = line.chars().skip(span.start.column).collect();
            span.end = LineColumn {
                line: span.start.line,
                column: span.start.column + rendered.chars().count() - 1,
            };
        }

        // TODO: cache character offsets to avoid repeated linear scans.

        let rendered_len = rendered.chars().count();

        log::trace!("extracted from source: >{rendered}< @ {span:?}");
        let (variant, span, pre, post) = detect_comment_variant(content, &rendered, span)?;

        let len_in_chars = rendered_len.saturating_sub(post + pre);

        if let Some(span_len) = span.one_line_len() {
            if log::log_enabled!(log::Level::Trace) {
                let extracted =
                    sub_chars(rendered.as_str(), pre..rendered_len.saturating_sub(post));
                log::trace!(target: "quirks", "{span:?} {pre}||{post} for \n extracted: >{extracted}<\n rendered:  >{rendered}<");
                assert_eq!(len_in_chars, span_len);
            }
        }

        let trimmed_literal = Self {
            variant,
            len_in_chars,

            rendered,
            span,
            pre,
            post,
            preferred_line_delimiter,
        };
        Ok(trimmed_literal)
    }
}

impl TrimmedLiteral {
    /// Construct a literal from single-line source text, syntax, delimiter lengths, and position.
    /// The caller supplies a one-based line, a zero-based character column, and ASCII delimiters.
    pub fn from(
        variant: CommentVariant,
        content: &str,
        pre: usize,
        post: usize,
        line: usize,
        column: usize,
    ) -> std::result::Result<TrimmedLiteral, String> {
        let content_chars_len = content.chars().count();
        let mut span = Span {
            start: LineColumn { line, column },
            end: LineColumn {
                line,
                column: column + content_chars_len,
            },
        };

        trim_span(content, &mut span, pre, post + 1);

        Ok(TrimmedLiteral {
            variant,
            span,
            rendered: content.to_string(),
            pre,
            post,
            len_in_chars: content_chars_len - pre - post,

            preferred_line_delimiter: source_line_with_delimiter(content, 1)
                .and_then(|(_, delimiter)| delimiter),
        })
    }
}

impl TrimmedLiteral {
    pub(crate) fn preferred_line_delimiter(&self) -> Option<&'static str> {
        self.preferred_line_delimiter.map(LineDelimiter::as_str)
    }

    /// Represent the rendered content as `str`.
    ///
    /// Does not contain `pre` and `post` characters.
    pub fn as_str(&self) -> &str {
        &self.rendered.as_str()[self.pre..(self.pre + self.len())]
    }

    /// The prefix characters.
    pub fn prefix(&self) -> &str {
        &self.rendered.as_str()[..self.pre]
    }

    /// The suffix characters.
    pub fn suffix(&self) -> &str {
        &self.rendered.as_str()[(self.pre + self.len())..]
    }

    /// Full representation including `prefix` and `postfix` characters.
    pub fn as_untrimmed_str(&self) -> &str {
        self.rendered.as_str()
    }

    /// Length in characters, excluding `pre` and `post`.
    pub fn len_in_chars(&self) -> usize {
        self.len_in_chars
    }

    /// Length in bytes, excluding `pre` and `post`.
    pub fn len(&self) -> usize {
        self.rendered.len().saturating_sub(self.pre + self.post)
    }

    /// Whether the content is empty, excluding `pre` and `post`.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Obtain the number of characters in `pre()`.
    ///
    /// Since all pre characters are ASCII, this is equivalent to the number of
    /// bytes in `pre()`.
    pub fn pre(&self) -> usize {
        self.pre
    }

    /// Obtain the number of characters in `post()`.
    ///
    /// Since all suffix characters are ASCII, this is equivalent to the number of
    /// bytes in `post()`.
    pub fn post(&self) -> usize {
        self.post
    }

    /// The span that is covered by this literal.
    ///
    /// Covers only the content, no marker or helper characters.
    pub fn span(&self) -> Span {
        self.span
    }

    /// Access the characters via an iterator.
    pub fn chars(&self) -> impl Iterator<Item = char> + '_ {
        self.as_str().chars()
    }

    /// The string variant type, see [`CommentVariant`](self::CommentVariant)
    /// for details.
    pub fn variant(&self) -> CommentVariant {
        self.variant.clone()
    }

    /// Display helper, mostly used for debug investigations
    #[allow(unused)]
    pub(crate) fn display(&self, highlight: Range) -> TrimmedLiteralDisplay<'_> {
        TrimmedLiteralDisplay::from((self, highlight))
    }
}

impl fmt::Debug for TrimmedLiteral {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use console::Style;

        let pick = Style::new().on_black().underlined().dim().cyan();
        let cutoff = Style::new().on_black().bold().dim().yellow();

        write!(
            formatter,
            "{}{}{}",
            cutoff.apply_to(&self.prefix()),
            pick.apply_to(&self.as_str()),
            cutoff.apply_to(&self.suffix()),
        )
    }
}

/// A display style wrapper for a trimmed literal.
///
/// Allows better display of coverage results without code duplication.
///
/// Consists of literal reference and a relative range to the start of the
/// literal.
#[derive(Debug, Clone)]
pub struct TrimmedLiteralDisplay<'a>(pub &'a TrimmedLiteral, pub Range);

impl<'a, R> From<(R, Range)> for TrimmedLiteralDisplay<'a>
where
    R: Into<&'a TrimmedLiteral>,
{
    fn from(tuple: (R, Range)) -> Self {
        let tuple0 = tuple.0.into();
        Self(tuple0, tuple.1)
    }
}

impl<'a> From<TrimmedLiteralDisplay<'a>> for (&'a TrimmedLiteral, Range) {
    fn from(val: TrimmedLiteralDisplay<'a>) -> Self {
        (val.0, val.1)
    }
}

impl<'a> fmt::Display for TrimmedLiteralDisplay<'a> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use console::Style;

        // part that is hidden by the trimmed literal, but still present in the actual literal
        let cutoff = Style::new().on_black().bold().underlined().yellow();
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
        let (pre, ctx1) = if start > literal.pre() {
            (
                // ok since that is ascii, so it's single bytes
                cutoff.apply_to(&data[..literal.pre()]).to_string(),
                {
                    let s = sub_chars(data, literal.pre()..start);
                    context.apply_to(s.as_str()).to_string()
                },
            )
        } else if start <= literal.len_in_chars() {
            let s = sub_chars(data, 0..start);
            (cutoff.apply_to(s.as_str()).to_string(), String::new())
        } else {
            (String::new(), "!!!".to_owned())
        };
        // highlight the given range
        let highlight = if end >= literal.len_in_chars() {
            let s = sub_chars(data, start..literal.len_in_chars());
            oob.apply_to(s.as_str()).to_string()
        } else {
            let s = sub_chars(data, start..end);
            highlight.apply_to(s.as_str()).to_string()
        };
        // color trailing context if any as well as the closing quote character
        let post_idx = literal.pre() + literal.len_in_chars();
        let (ctx2, post) = if post_idx > end {
            let s_ctx = sub_chars(data, end..post_idx);
            let s_cutoff = sub_chars(data, post_idx..literal.len_in_chars());
            (
                context.apply_to(s_ctx.as_str()).to_string(),
                cutoff.apply_to(s_cutoff.as_str()).to_string(),
            )
        } else if end < literal.len_in_chars() {
            let s = sub_chars(
                data,
                end..(literal.len_in_chars() + literal.pre() + literal.post()),
            );
            (String::new(), cutoff.apply_to(s.as_str()).to_string())
        } else {
            (String::new(), oob.apply_to("!!!").to_string())
        };

        write!(formatter, "{pre}{ctx1}{highlight}{ctx2}{post}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testcase::annotated_literals_raw;
    use assert_matches::assert_matches;

    #[test]
    fn trimmed_content_is_empty() {
        for (content, pre, post, expected_empty) in [("/**/", 2, 2, true), ("/*🌡*/", 2, 2, false)]
        {
            let literal =
                TrimmedLiteral::from(CommentVariant::SlashAsterisk, content, pre, post, 1, 0)
                    .unwrap();
            assert_eq!(literal.is_empty(), expected_empty);
            assert_eq!(literal.is_empty(), literal.as_str().is_empty());
            assert_eq!(literal.len(), literal.as_str().len());
        }
    }

    fn assert_chunk(
        source: &str,
        chunk: &crate::CheckableChunk,
        expected: &str,
        fragments: &[(crate::Range, Span)],
    ) {
        assert_eq!(chunk.as_str(), expected, "source: {source:?}");
        let actual: Vec<_> = chunk
            .iter()
            .map(|(range, span)| (range.clone(), *span))
            .collect();
        assert_eq!(actual, fragments, "source: {source:?}");
        for (range, span) in fragments {
            // Empty fragments use a reversed inclusive span, not source text.
            if range.is_empty() {
                continue;
            }
            assert_eq!(
                util::load_span_from(source.as_bytes(), *span).unwrap(),
                sub_chars(chunk.as_str(), range.clone()),
            );
            assert_eq!(
                chunk
                    .find_spans(range.clone())
                    .into_iter()
                    .collect::<Vec<_>>(),
                vec![(range.clone(), *span)],
            );
        }
    }

    fn line_span(line: usize, start: usize, end: usize) -> Span {
        Span {
            start: LineColumn {
                line,
                column: start,
            },
            end: LineColumn { line, column: end },
        }
    }

    fn doc_chunks(source: &str) -> Vec<crate::CheckableChunk> {
        crate::Clusters::load_from_str(source, true, false)
            .unwrap()
            .into()
    }

    #[test]
    fn line_comment_endings_preserve_exact_content_and_spans() {
        for prefix in ["///", "//!"] {
            for first_ending in ["\n", "\r\n"] {
                for second_ending in ["\n", "\r\n"] {
                    for eof_ending in ["", "\n", "\r\n"] {
                        let source = format!(
                            "{prefix} Préface 🦀.{first_ending}{prefix}{second_ending}{prefix} Last.  {eof_ending}"
                        );
                        let chunks = doc_chunks(&source);
                        assert_eq!(chunks.len(), 1);
                        assert_eq!(chunks[0].preferred_line_delimiter(), Some(first_ending));
                        assert_chunk(
                            &source,
                            &chunks[0],
                            " Préface 🦀.\n\n Last.  ",
                            &[
                                (0..11, line_span(1, 3, 13)),
                                (12..12, line_span(2, 3, 2)),
                                (13..21, line_span(3, 3, 10)),
                            ],
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn eof_line_comment_after_code_has_no_physical_terminator() {
        for ending in ["\n", "\r\n"] {
            for eof_ending in ["", "\n", "\r\n"] {
                let source = format!("struct X;{ending}/// last{eof_ending}");
                let chunks = doc_chunks(&source);
                assert_eq!(chunks.len(), 1);
                let preferred = if eof_ending.is_empty() {
                    ending
                } else {
                    eof_ending
                };
                assert_eq!(chunks[0].preferred_line_delimiter(), Some(preferred));
                assert_chunk(&source, &chunks[0], " last", &[(0..5, line_span(2, 3, 7))]);
            }
        }
    }

    #[test]
    fn empty_inner_block_comment_does_not_discard_valid_documentation() {
        for ending in ["\n", "\r\n"] {
            let source = format!("/*!*/{ending}//! Module.{ending}/// Item.{ending}struct X;");
            let chunks = doc_chunks(&source);
            assert_eq!(chunks.len(), 2);
            for chunk in &chunks {
                assert_eq!(chunk.preferred_line_delimiter(), Some(ending));
            }
            assert_chunk(
                &source,
                &chunks[0],
                "\n Module.",
                &[(0..0, line_span(1, 3, 2)), (1..9, line_span(2, 3, 10))],
            );
            assert_chunk(&source, &chunks[1], " Item.", &[(0..6, line_span(3, 3, 8))]);
        }
        for ending in ["", "\n", "\r\n"] {
            let source = format!("/*!*/{ending}");
            let chunks = doc_chunks(&source);
            assert_eq!(chunks.len(), 1);
            assert_chunk(&source, &chunks[0], "", &[(0..0, line_span(1, 3, 2))]);
            assert_eq!(
                chunks[0].preferred_line_delimiter(),
                if ending.is_empty() {
                    None
                } else {
                    Some(ending)
                },
            );
        }
    }

    #[test]
    fn crlf_string_literals_keep_escapes_and_multiline_source_spans() {
        let source = "#[doc = \"one\\ntwo\"]\r\nstruct X;\r\n";
        let chunks = doc_chunks(source);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].preferred_line_delimiter(), Some("\r\n"));
        assert_chunk(
            source,
            &chunks[0],
            "one\\ntwo",
            &[(0..8, line_span(1, 9, 16))],
        );

        let source = "#[doc = r#\"Café\r\n🦀\"#]\r\nstruct X;\r\n";
        let chunks = doc_chunks(source);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].preferred_line_delimiter(), Some("\r\n"));
        assert_chunk(
            source,
            &chunks[0],
            "Café\r\n🦀",
            &[(
                0..7,
                Span {
                    start: LineColumn {
                        line: 1,
                        column: 11,
                    },
                    end: LineColumn { line: 2, column: 0 },
                },
            )],
        );
    }

    #[test]
    fn single_line_comments_keep_newline_metadata_without_content_newlines() {
        let mut literals = Vec::new();
        for ending in ["", "\n", "\r\n"] {
            let source = format!("/// Words that need wrapping.{ending}");
            let literal = annotated_literals_raw(&source).next().unwrap();
            let literal = TrimmedLiteral::load_from(&source, Span::from(literal.span())).unwrap();
            let preferred = if ending.is_empty() {
                None
            } else {
                Some(ending)
            };
            assert_eq!(literal.preferred_line_delimiter(), preferred);
            assert_eq!(literal.clone().preferred_line_delimiter(), preferred);
            let chunk = crate::LiteralSet::from(literal.clone()).into_chunk();
            assert_eq!(chunk.preferred_line_delimiter(), preferred);
            assert_eq!(chunk.clone().preferred_line_delimiter(), preferred);
            assert_chunk(
                &source,
                &chunk,
                " Words that need wrapping.",
                &[(0..26, line_span(1, 3, 28))],
            );
            literals.push(literal);
        }
        assert_ne!(literals[0], literals[1]);
        assert_ne!(literals[1], literals[2]);
    }

    #[test]
    fn eof_comment_inherits_nearest_not_majority_source_delimiter() {
        let source = "struct A;\r\nstruct B;\r\nstruct C;\n/// Last.";
        let chunks = doc_chunks(source);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].preferred_line_delimiter(), Some("\n"));
        assert_chunk(source, &chunks[0], " Last.", &[(0..6, line_span(4, 3, 8))]);
    }

    #[test]
    fn conditional_chunks_prefer_the_first_selected_literals_source_style() {
        let source = "#[cfg_attr(flag, doc = \"Conditional.\")]\r\n/// Shared.\nstruct X;";
        let chunks = doc_chunks(source);
        assert_eq!(chunks.len(), 2);
        for chunk in chunks {
            match chunk.as_str() {
                " Shared." => {
                    assert_eq!(chunk.preferred_line_delimiter(), Some("\n"));
                    assert_chunk(source, &chunk, " Shared.", &[(0..8, line_span(2, 3, 10))]);
                }
                "Conditional.\n Shared." => {
                    assert_eq!(chunk.preferred_line_delimiter(), Some("\r\n"));
                    assert_chunk(
                        source,
                        &chunk,
                        "Conditional.\n Shared.",
                        &[(0..12, line_span(1, 24, 35)), (13..21, line_span(2, 3, 10))],
                    );
                }
                other => panic!("unexpected projection: {other:?}"),
            }
        }
    }

    #[test]
    fn variant_detect() {
        let content = r###"#[doc=r"foo"]"###.to_owned();
        let rendered = r##"r"foo""##.to_owned();
        assert_matches!(
        detect_comment_variant(content.as_str(), &rendered, Span{
            start: LineColumn {
                line: 1,
                column: 6,
            },
            end: LineColumn {
                line: 1,
                column: 12 + 1,
            },
        }), Ok((CommentVariant::MacroDocEqStr(prefix, n_pounds), _, _, _)) => {
            assert_eq!(n_pounds, 1);
            assert_eq!(prefix, "#[doc=");
        });
    }

    macro_rules! block_comment_test {
        ($name:ident, $content:literal) => {
            #[test]
            fn $name() {
                const CONTENT: &str = $content;
                let mut literals = annotated_literals_raw(CONTENT);
                let literal = literals.next().unwrap();
                assert!(literals.next().is_none());

                let tl = TrimmedLiteral::load_from(CONTENT, Span::from(literal.span())).unwrap();
                assert!(CONTENT.starts_with(tl.prefix()));
                assert!(CONTENT.ends_with(tl.suffix()));
                assert_eq!(
                    CONTENT
                        .chars()
                        .skip(tl.pre())
                        .take(tl.len_in_chars())
                        .collect::<String>(),
                    tl.as_str().to_owned()
                )
            }
        };
    }

    block_comment_test!(trimmed_oneline_doc, "/** dooc */");
    block_comment_test!(trimmed_oneline_mod, "/*! dooc */");

    block_comment_test!(
        trimmed_multi_doc,
        "/**
mood
*/"
    );
    block_comment_test!(
        trimmed_multi_mod,
        "/*!
mood
*/"
    );
}
