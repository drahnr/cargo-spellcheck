use crate::util::{self, sub_chars};
use crate::{Range, Span};
use anyhow::{bail, Result};

use fancy_regex::Regex;
use std::convert::TryFrom;
use std::fmt;

/// Track what kind of comment the literal is
#[derive(Debug, Clone, Copy, Hash)]
#[non_exhaustive]
pub enum CommentVariant {
    /// `///`
    TripleSlash,
    /// `#[doc=`
    MacroDocEq,
}

/// A literal with meta info where the first and list whitespace may be found.
#[derive(Clone)]
pub struct TrimmedLiteral {
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
        // itt's unclear why the trailing `]` character is part of the given span, it shout not be part
        // of it, but the span we obtain from literal seems to be wrong, adding one trailing char.

        // Either cut off `]` or `\n` - we don't need either.
        span.end.column = span.end.column.saturating_sub(1);

        let rendered = util::load_span_from(content.as_bytes(), span.clone())?;
        // TODO cache the offsets for faster processing and avoiding repeated O(n) ops
        // let byteoffset2char = rendered.char_indices().enumerate().collect::<indexmap::IndexMap<usize, (usize, char)>>();
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

            (CommentVariant::TripleSlash, span, pre, post)
        } else {
            // pre and post are for the rendered content
            // not necessarily for the span

            //^r(#+?)"(?:.*\s*)+(?=(?:"\1))("\1)$
            lazy_static::lazy_static! {
                static ref BOUNDED_RAW_STR: Regex = Regex::new(r##"^(r(#+)?")(?:.*\s*)+?(?=(?:"\2))("\2)\s*\]?\s*$"##).expect("BOUNEDED_RAW_STR regex compiles");
                static ref BOUNDED_STR: Regex = Regex::new(r##"^"(?:.(?!"\\"))*?"*\s*\]?\s*"$"##).expect("BOUNEDED_STR regex compiles");
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
                debug_assert_eq!('"', rendered.as_bytes()[0usize] as char);
                debug_assert_eq!('"', rendered.as_bytes()[rendered.len() - 1usize] as char);
                (pre, post)
            } else {
                bail!("Regex should match >{}<", rendered);
            };

            span.start.column += pre;
            span.end.column = span.end.column.saturating_sub(post);

            (CommentVariant::MacroDocEq, span, pre, post)
        };

        let len_in_chars = rendered_len.saturating_sub(post + pre);

        if let Some(span_len) = span.one_line_len() {
            let extracted = sub_chars(rendered.as_str(), pre..rendered_len.saturating_sub(post));
            log::trace!(target: "quirks", "{:?} {}||{} for \n extracted: >{}<\n rendered:  >{}<", span, pre, post, &extracted, rendered);
            assert_eq!(len_in_chars, span_len);
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
    /// Since all pre charcters are ascii, this is equivalent to the number of bytes in `pre()`.
    pub fn pre(&self) -> usize {
        self.pre
    }

    /// Obtain the number of characters in `post()`.
    ///
    /// Since all pre charcters are ascii, this is equivalent to the number of bytes in `post()`.
    pub fn post(&self) -> usize {
        self.post
    }

    /// The span that is covered by this literal.
    ///
    /// Covers only the content, no marker or helper chracters.
    pub fn span(&self) -> Span {
        self.span.clone()
    }

    /// Access the characters via an iterator.
    pub fn chars<'a>(&'a self) -> impl Iterator<Item = char> + 'a {
        self.as_str().chars()
    }

    /// The string variant type, see [`CommentVariant`](self::CommentVariant) for details.
    pub fn variant(&self) -> CommentVariant {
        self.variant
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::util::load_span_from;
    use crate::LineColumn;

    pub(crate) fn annotated_literals(source: &str) -> Vec<TrimmedLiteral> {
        let stream =
            syn::parse_str::<proc_macro2::TokenStream>(source).expect("Must be valid rust");
        stream
            .into_iter()
            .filter_map(|x| {
                if let proc_macro2::TokenTree::Group(group) = x {
                    Some(group.stream().into_iter())
                } else {
                    None
                }
            })
            .flatten()
            .filter_map(|x| {
                if let proc_macro2::TokenTree::Literal(literal) = x {
                    Some(literal)
                } else {
                    None
                }
            })
            .map(|literal| {
                TrimmedLiteral::try_from((source, literal))
                    .expect("Literals must be convertable to trimmed literals")
            })
            .collect()
    }

    const PREFIX_RAW_LEN: usize = 3;
    const SUFFIX_RAW_LEN: usize = 2;
    const GAENSEFUESSCHEN: usize = 1;

    #[derive(Clone, Debug)]
    struct Triplet {
        /// source content
        source: &'static str,
        /// expected doc comment content without modifications
        extracted: &'static str,
        /// expected doc comment content after applying trimming rules
        trimmed: &'static str,
        /// expected span as extracted by proc_macro2
        extracted_span: Span,
        /// trimmed span, so it is aligned with the proper doc comment
        trimmed_span: Span,
    }

    const TEST_DATA: &[Triplet] = &[
        // 0
        Triplet {
            source: r#"
/// One Doc
struct One;
"#,
            extracted: r#"" One Doc""#,
            trimmed: " One Doc",
            extracted_span: Span {
                start: LineColumn {
                    line: 2usize,
                    column: 0,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 10usize,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 2usize,
                    column: 3usize,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 10usize,
                },
            },
        },
        // 1
        Triplet {
            source: r##"
    ///meanie
struct Meanie;
"##,
            extracted: r#""meanie""#,
            trimmed: "meanie",
            extracted_span: Span {
                start: LineColumn {
                    line: 2usize,
                    column: 0usize + 7 - PREFIX_RAW_LEN,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 0usize + 12 + SUFFIX_RAW_LEN,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 2usize,
                    column: 0usize + 7,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 0usize + 12,
                },
            },
        },
        // 2
        Triplet {
            source: r#"
#[doc = "Two Doc"]
struct Two;
"#,
            extracted: r#""Two Doc""#,
            trimmed: "Two Doc",
            extracted_span: Span {
                start: LineColumn {
                    line: 2usize,
                    column: 0usize + 10 - GAENSEFUESSCHEN,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 6usize + 10 + GAENSEFUESSCHEN,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 2usize,
                    column: 0usize + 9,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 6usize + 9,
                },
            },
        },
        // 3
        Triplet {
            source: r##"
    #[doc=r#"Three Doc"#]
struct Three;
"##,
            extracted: r###"r#"Three Doc"#"###,
            trimmed: "Three Doc",
            extracted_span: Span {
                start: LineColumn {
                    line: 2usize,
                    column: 4usize + 11,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 13usize + 11,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 2usize,
                    column: 0usize + 13,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 13usize + 8,
                },
            },
        },
        // 4
        Triplet {
            source: r###"
#[doc = r##"Four
has
multiple
lines
"##]
struct Four;
"###,
            extracted: r###"r##"Four
has
multiple
lines
"##"###,
            trimmed: r#"Four
has
multiple
lines
"#,
            extracted_span: Span {
                start: LineColumn {
                    line: 2usize,
                    column: 12usize - 4,
                },
                end: LineColumn {
                    line: 6usize,
                    column: 0usize + 3,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 2usize,
                    column: 12usize,
                },
                end: LineColumn {
                    line: 6usize,
                    column: 0usize,
                },
            },
        },
        // 5
        Triplet {
            source: r###"
#[doc        ="XYZ"]
struct Five;
"###,
            extracted: r#""XYZ""#,
            trimmed: r#"XYZ"#,
            extracted_span: Span {
                start: LineColumn {
                    line: 2usize,
                    column: 15usize - GAENSEFUESSCHEN,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 15usize + 2 + GAENSEFUESSCHEN,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 2usize,
                    column: 15usize,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 15usize + 2,
                },
            },
        },
        // 6
        Triplet {
            source: r#"

    /// if a layer is provided a identiacla "input" and "output", it will only be supplied an
    fn compute_in_place(&self) -> bool {
        false
    }

"#,
            extracted: r#"" if a layer is provided a identiacla "input" and "output", it will only be supplied an""#,
            trimmed: r#" if a layer is provided a identiacla "input" and "output", it will only be supplied an"#,
            extracted_span: Span {
                start: LineColumn {
                    line: 3usize,
                    column: 7usize - GAENSEFUESSCHEN,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 92usize + GAENSEFUESSCHEN,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 3usize,
                    column: 7usize,
                },
                end: LineColumn {
                    line: 3usize,
                    column: 92usize,
                },
            },
        },
        // 7
        Triplet {
            source: r#"

/// 🍉 ← αA<sup>OP</sup>x + βy
fn unicode(&self) -> bool {
    true
}

"#,
            extracted: r#"" 🍉 ← αA<sup>OP</sup>x + βy""#,
            trimmed: r#" 🍉 ← αA<sup>OP</sup>x + βy"#,
            extracted_span: Span {
                start: LineColumn {
                    line: 3usize,
                    column: 3usize - GAENSEFUESSCHEN,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 28usize + GAENSEFUESSCHEN,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 3usize,
                    column: 3usize,
                },
                end: LineColumn {
                    line: 3usize,
                    column: 28usize,
                },
            },
        },
    ];

    fn comment_variant_span_range_validation(index: usize) {
        let _ = env_logger::builder()
            .filter(None, log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        let triplet = TEST_DATA[index].clone();
        let literals = annotated_literals(triplet.source);

        assert_eq!(literals.len(), 1);

        let literal = literals.first().expect("Must contain exactly one literal");

        assert_eq!(literal.as_str(), triplet.trimmed);

        // just for better visual errors
        let excerpt = load_span_from(triplet.source.as_bytes(), literal.span()).unwrap();
        let expected_excerpt =
            load_span_from(triplet.source.as_bytes(), triplet.trimmed_span).unwrap();
        assert_eq!(excerpt, expected_excerpt);

        assert_eq!(literal.span(), triplet.trimmed_span);
    }

    #[test]
    fn raw_variant_0_triple_slash() {
        comment_variant_span_range_validation(0);
    }

    #[test]
    fn raw_variant_1_spaces_triple_slash() {
        comment_variant_span_range_validation(1);
    }

    #[test]
    fn raw_variant_2_spaces_doc_eq_single_quote() {
        comment_variant_span_range_validation(2);
    }

    #[test]
    fn raw_variant_3_doc_eq_single_r_hash_quote() {
        comment_variant_span_range_validation(3);
    }

    #[test]
    fn raw_variant_4_doc_eq_multi() {
        comment_variant_span_range_validation(4);
    }

    #[test]
    fn raw_variant_5_doc_spaces_eq_single_quote() {
        comment_variant_span_range_validation(5);
    }

    #[test]
    fn raw_variant_6_quote_chars() {
        comment_variant_span_range_validation(6);
    }

    #[test]
    fn raw_variant_7_unicode_symbols() {
        comment_variant_span_range_validation(7);
    }
}
