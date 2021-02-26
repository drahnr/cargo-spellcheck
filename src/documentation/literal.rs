use crate::util::{self, sub_chars};
use crate::{Range, Span};
use anyhow::{bail, Result};

use fancy_regex::Regex;
use lazy_static::lazy_static;
use proc_macro2::LineColumn;
use std::convert::TryFrom;
use std::fmt;

/// Track what kind of comment the literal is
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
    /// `#[doc=` with actual prefix like `#[doc=` and
    /// the total length of `r###` etc. including `r`
    /// but without `"`
    MacroDocEq(String, usize),
    /// Commonmark File
    CommonMark,
    /// Developer line comment
    DoubleSlash,
    /// Developer block comment
    SlashStar,
    /// Unknown Variant
    Unknown,
}

impl Default for CommentVariant {
    fn default() -> Self {
        CommentVariant::Unknown
    }
}

impl CommentVariant {
    /// Return the prefix string.
    ///
    /// Does not include whitespaces for `///` and `//!` variants!
    pub fn prefix_string(&self) -> String {
        match self {
            CommentVariant::TripleSlash => "///".into(),
            CommentVariant::DoubleSlashEM => "//!".into(),
            CommentVariant::MacroDocEq(d, p) => {
                let raw = match p {
                    // TODO: make configureable if each line will start with #[doc ="
                    // TODO: but not here!
                    0 => "\"".to_owned(),
                    x => format!("r{}\"", "#".repeat(x.saturating_sub(1))),
                };
                format!(r#"{}{}"#, d, raw)
            }
            CommentVariant::CommonMark => "".to_string(),
            CommentVariant::DoubleSlash => "//".to_string(),
            CommentVariant::SlashStar => "/*".to_string(),
            CommentVariant::SlashAsterisk => "/*".to_string(),
            CommentVariant::SlashAsteriskEM => "/*!".to_string(),
            CommentVariant::SlashAsteriskAsterisk => "/**".to_string(),
            unhandled => unreachable!(
                "String representation for comment variant {:?} exists. qed",
                unhandled
            ),
        }
    }
    /// Return length (in bytes) of comment prefix for each variant.
    ///
    /// By definition matches the length of `prefix_string`.
    pub fn prefix_len(&self) -> usize {
        match self {
            CommentVariant::TripleSlash | CommentVariant::DoubleSlashEM => 3,
            CommentVariant::MacroDocEq(d, p) => d.len() + *p + 1,
            _ => self.prefix_string().len(),
        }
    }

    /// Return suffix of different comment variants
    pub fn suffix_len(&self) -> usize {
        match self {
            CommentVariant::MacroDocEq(_, 0) => 2,
            CommentVariant::MacroDocEq(_, p) => p + 1,
            CommentVariant::SlashAsteriskAsterisk
            | CommentVariant::SlashAsteriskEM
            | CommentVariant::SlashAsterisk => 2,
            _ => 0,
        }
    }

    /// Return string which will be appended to each line
    pub fn suffix_string(&self) -> String {
        match self {
            CommentVariant::MacroDocEq(_, p) if p == 0 || p == 1 => r#""]"#.to_string(),
            CommentVariant::MacroDocEq(_, p) => {
                r#"""#.to_string() + &"#".repeat(n.saturating_sub(1)) + "]"
            }
            CommentVariant::SlashAsteriskAsterisk
            | CommentVariant::SlashAsteriskEM
            | CommentVariant::SlashAsterisk => "*/".to_string(),
            _ => "".to_string(),
        }
    }
}

/// A literal with meta info where the first and list whitespace may be found.
#[derive(Clone)]
pub struct TrimmedLiteral {
    /// Track what kind of comment the literal is
    variant: CommentVariant,
    /// The span of rendered content, minus pre and post already applied.
    span: Span,
    /// the complete rendered string including post and pre.
    rendered: String,
    /// Literal prefix length.
    pre: usize,
    /// Literal postfix length.
    post: usize,
    /// Length of rendered **minus** `pre` and `post` in UTF-8 characters.
    len_in_chars: usize,
    len_in_bytes: usize,
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
        self.len_in_bytes.hash(hasher);
        self.len_in_chars.hash(hasher);
    }
}

impl TryFrom<(&str, proc_macro2::Literal)> for TrimmedLiteral {
    type Error = anyhow::Error;
    fn try_from((content, literal): (&str, proc_macro2::Literal)) -> Result<Self> {
        // let rendered = literal.to_string();
        // produces pretty unusable garabage, since it modifies the content of `///`
        // comments which could contain " which will be escaped
        // and therefor cause the `span()` to yield something that does
        // not align with the rendered literal at all and there are too
        // many pitfalls to sanitize all cases, so reading given span
        // from the file again, and then determining its type is way safer.

        let mut span = Span::from(literal.span());
        // It's unclear why the trailing `]` character is part of the given span, it shout not be part
        // of it, but the span we obtain from literal seems to be wrong, adding one trailing char.

        // Either cut off `]` or `\n` - we don't need either.
        span.end.column = span.end.column.saturating_sub(1);

        // If the line ending has more than one character, we have to account
        // for that. Otherwise cut of the last character of the ending such that
        // we can't properly detect them anymore.
        if crate::reflow::extract_delimiter(content)
            .unwrap_or("\n")
            .len()
            > 1
        {
            log::trace!(target: "documentation", "Found two character line ending like CRLF");
            span.end.column += 1;
        }

        let rendered = util::load_span_from(content.as_bytes(), span.clone())?;
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

        // TODO cache the offsets for faster processing and avoiding repeated O(n) ops
        // let byteoffset2char = rendered.char_indices().enumerate().collect::<indexmap::IndexMap<_usize, (_usize, char)>>();
        // let rendered_len = byteoffset2char.len();

        let rendered_len = rendered.chars().count();

        log::trace!("extracted from source: >{}< @ {:?}", rendered, span);
        let (variant, span, pre, post) = if rendered.starts_with("///")
            || rendered.starts_with("//!")
        {
            let pre = 3; // `///`
            let post = 0; // trailing `\n` is already accounted for above

            span.start.column += pre;

            // must always be a single line
            assert_eq!(span.start.line, span.end.line);
            // if the line includes quotes, the rustc converts them internally
            // to `#[doc="content"]`, where - if `content` contains `"` will substitute
            // them as `\"` which will inflate the number columns.
            // Since we can not distinguish between orignally escaped, we simply
            // use the content read from source.

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
            span.start.column += pre;
            span.end.column = dbg!(span.end.column).saturating_sub(post);

            (variant, span, pre, post)
        } else {
            // pre and post are for the rendered content
            // not necessarily for the span

            //^r(#+?)"(?:.*\s*)+(?=(?:"\1))("\1)$
            lazy_static! {
                static ref BOUNDED_RAW_STR: Regex =
                    Regex::new(r##"^(r(#+)?")(?:.*\s*)+?(?=(?:"\2))("\2)\s*\]?\s*$"##)
                        .expect("BOUNEDED_RAW_STR regex compiles");
                static ref BOUNDED_STR: Regex = Regex::new(r##"^"(?:.(?!"\\"))*?"*\s*\]?\s*"$"##)
                    .expect("BOUNEDED_STR regex compiles");
            };

            let (pre, post) = if let Some(captures) =
                BOUNDED_RAW_STR.captures(rendered.as_str()).ok().flatten()
            {
                log::trace!("raw str: >{}<", rendered.as_str());
                let pre = if let Some(prefix) = captures.get(1) {
                    log::trace!("raw str pre: >{}<", prefix.as_str());
                    prefix.as_str().len()
                } else {
                    bail!("Should have a raw str pre match with a capture group");
                };
                let post = if let Some(suffix) = captures.get(captures.len() - 1) {
                    log::trace!("raw str post: >{}<", suffix.as_str());
                    suffix.as_str().len()
                } else {
                    bail!("Should have a raw str post match with a capture group");
                };

                // r####" must match "####
                debug_assert_eq!(pre, post + 1);

                (pre, post)
            } else if let Some(_captures) = BOUNDED_STR.captures(rendered.as_str()).ok().flatten() {
                // r####" must match "####
                let pre = 1;
                let post = 1;
                debug_assert_eq!('"', rendered.as_bytes()[0_usize] as char);
                debug_assert_eq!('"', rendered.as_bytes()[rendered.len() - 1_usize] as char);
                (pre, post)
            } else {
                bail!("Regex should match >{}<", rendered);
            };

            span.start.column += pre;
            span.end.column = span.end.column.saturating_sub(post);

            (
                CommentVariant::MacroDocEq(prefix, pre.saturating_sub(1)),
                span,
                pre,
                post,
            )
        };

        let len_in_chars = rendered_len.saturating_sub(post + pre);

        if let Some(span_len) = span.one_line_len() {
            if log::log_enabled!(log::Level::Trace) {
                let extracted =
                    sub_chars(rendered.as_str(), pre..rendered_len.saturating_sub(post));
                log::trace!(target: "quirks", "{:?} {}||{} for \n extracted: >{}<\n rendered:  >{}<", span, pre, post, &extracted, rendered);
                assert_eq!(len_in_chars, span_len);
            }
        }

        let len_in_bytes = rendered.len().saturating_sub(post + pre);
        let literal = Self {
            variant,
            len_in_chars,
            len_in_bytes,
            rendered,
            span,
            pre,
            post,
        };
        Ok(literal)
    }
}

impl TrimmedLiteral {
    /// Creates a new (single line) literal from the variant, the content, the size of the
    /// pre & post and the line/column on which it starts. Fails if provided with multiline content
    /// (i.e. if the content contains a line-break).
    pub fn from(
        variant: CommentVariant,
        content: &str,
        pre: usize,
        post: usize,
        line: usize,
        column: usize,
    ) -> Result<TrimmedLiteral, String> {
        if content.contains("\n") {
            return Err("Cannot create a multiline trimmed literal".to_string());
        }
        let pre_text = &content[..pre];
        let post_text = &content[content.len() - post..];
        Ok(TrimmedLiteral {
            variant,
            span: Span {
                start: LineColumn {
                    line,
                    column: column + pre_text.chars().count(),
                },
                end: LineColumn {
                    line,
                    column: column + content.chars().count() - post_text.chars().count() - 1,
                },
            },
            rendered: content.to_string(),
            pre,
            post,
            len_in_chars: content.chars().count()
                - pre_text.chars().count()
                - post_text.chars().count(),
            len_in_bytes: content.len() - pre - post,
        })
    }
}

impl TrimmedLiteral {
    /// Represent the rendered content as `str`.
    ///
    /// Does not contain `pre` and `post` characters.
    pub fn as_str(&self) -> &str {
        &self.rendered.as_str()[self.pre..(self.pre + self.len_in_bytes)]
    }

    /// The prefix characters.
    pub fn prefix(&self) -> &str {
        &self.rendered.as_str()[..self.pre]
    }

    /// The suffix characters.
    pub fn suffix(&self) -> &str {
        &self.rendered.as_str()[(self.pre + self.len_in_bytes)..]
    }

    /// Full representation including `prefix` and `postfix` characters.
    pub fn as_untrimmed_str(&self) -> &str {
        &self.rendered.as_str()
    }

    /// Length in characters, excluding `pre` and `post`.
    pub fn len_in_chars(&self) -> usize {
        self.len_in_chars
    }

    /// Length in bytes, excluding `pre` and `post`.
    pub fn len(&self) -> usize {
        self.len_in_bytes
    }

    /// Obtain the number of characters in `pre()`.
    ///
    /// Since all pre characters are ASCII, this is equivalent to the number of bytes in `pre()`.
    pub fn pre(&self) -> usize {
        self.pre
    }

    /// Obtain the number of characters in `post()`.
    ///
    /// Since all pre characters are ASCII, this is equivalent to the number of bytes in `post()`.
    pub fn post(&self) -> usize {
        self.post
    }

    /// The span that is covered by this literal.
    ///
    /// Covers only the content, no marker or helper characters.
    pub fn span(&self) -> Span {
        self.span.clone()
    }

    /// Access the characters via an iterator.
    pub fn chars<'a>(&'a self) -> impl Iterator<Item = char> + 'a {
        self.as_str().chars()
    }

    /// The string variant type, see [`CommentVariant`](self::CommentVariant) for details.
    pub fn variant(&self) -> CommentVariant {
        self.variant.clone()
    }

    /// Display helper, mostly used for debug investigations
    #[allow(unused)]
    pub(crate) fn display(&self, highlight: Range) -> TrimmedLiteralDisplay {
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
/// Consists of literal reference and a relative range to the start of the literal.
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

impl<'a> Into<(&'a TrimmedLiteral, Range)> for TrimmedLiteralDisplay<'a> {
    fn into(self) -> (&'a TrimmedLiteral, Range) {
        (self.0, self.1)
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

        write!(formatter, "{}{}{}{}{}", pre, ctx1, highlight, ctx2, post)
    }
}
