use crate::{Range, Span};
use anyhow::{bail, Result};
use crate::util;

use regex::Regex;
use std::convert::TryFrom;
use std::fmt;

/// A literal with meta info where the first and list whitespace may be found.
#[derive(Clone)]
pub struct TrimmedLiteral {
    /// The span of rendered content, minus pre and post already applied.
    span: Span,
    /// the complete rendered string including post and pre.
    rendered: String,
    /// Literal prefx
    pub pre: usize,
    /// Literal postfix
    pub post: usize,
    /// Length of rendered **minus** `pre` and `post`.
    /// If the literal is all empty, `pre` and `post` become `0`, and `len` covers the full length of `rendered`.
    len: usize,
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
        self.rendered.hash(hasher);
        self.span.hash(hasher);
        self.pre.hash(hasher);
        self.post.hash(hasher);
        self.len.hash(hasher);
    }
}

impl TryFrom<(&str, proc_macro2::Literal)> for TrimmedLiteral {
    type Error = anyhow::Error;
    fn try_from((content, literal): (&str, proc_macro2::Literal)) -> Result<Self> {

        // pretty unusable garabage, since it modifies the content of `///`
        // comments which could contain " which will be escaped
        // and therefor cause the `span()` to yield something that does
        // not align with the rendered literal at all and there are too
        // many pitfalls to sanitize all cases
        // let rendered = literal.to_string();

        lazy_static::lazy_static! {
            static ref BOUNDED: Regex = Regex::new(r##"^(\s*(?:r#*)?").*("#*\s*)$"##).unwrap();
        };

        let mut span = Span::from(literal.span());
        let rendered = dbg!(util::load_span_from(content.as_bytes(), span.clone())?);


        log::trace!("extracted from source: >{}< @ {:?}", rendered, span);
        let (extracted, pre, post) = if rendered.starts_with("///") || rendered.starts_with("//!") {
            let pre = 3; // `///`
            let post = 1; // trailing `\n`
            // but the newline is not part of this
            (&rendered[pre..(rendered.len()-post)], pre, post)
        } else {
            // pre and post are for the rendered content
            // not necessarily for the span
            let (pre, post) = if let Some(captures) = BOUNDED.captures(rendered.as_str()) {
                let pre = if let Some(prefix) = captures.get(1) {
                    dbg!(prefix.as_str()).len()
                } else {
                    bail!("Should have a pre match with a capture group");
                };
                let post = if let Some(suffix) = captures.get(captures.len() - 1) {
                    dbg!(suffix.as_str()).len()
                } else {
                    bail!("Should have a post match with a capture group");
                };
                (pre, post)
            } else {
                bail!("Should match >{}<", rendered);
            };
            (&rendered[pre..(dbg!(rendered.len()) - dbg!(post))], pre, post)
        };

        let len = extracted.chars().count();

        span.start.column += pre;
        span.end.column -= post;


        if let Some(span_len) = span.one_line_len() {
            log::trace!(target: "quirks", "{:?} {}||{} for \n extracted: >{}<\n rendered:  >{}<", span, pre, post, &extracted, rendered);
            assert_eq!(len, span_len);
        }

        let literal = Self {
            len,
            rendered,
            span,
            pre,
            post,
        };
        Ok(literal)
    }
}

impl TrimmedLiteral {
    pub fn as_str(&self) -> &str {
        &self.rendered.as_str()[self.pre..(self.pre + self.len)]
    }
    pub fn prefix(&self) -> &str {
        &self.rendered.as_str()[..self.pre]
    }
    pub fn suffix(&self) -> &str {
        &self.rendered.as_str()[(self.pre + self.len)..]
    }

    pub fn as_untrimmed_str(&self) -> &str {
        &self.rendered.as_str()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn pre(&self) -> usize {
        self.pre
    }

    pub fn post(&self) -> usize {
        self.post
    }

    pub fn span(&self) -> Span {
        self.span.clone()
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
                cutoff.apply_to(&data[..literal.pre()]).to_string(),
                context.apply_to(&data[literal.pre()..start]).to_string(),
            )
        } else if start <= data.len() {
            (cutoff.apply_to(&data[..start]).to_string(), String::new())
        } else {
            (String::new(), "!!!".to_owned())
        };
        // highlight the given range
        let highlight = if end >= data.len() {
            oob.apply_to(&data[start..data.len()]).to_string()
        } else {
            highlight.apply_to(&data[start..end]).to_string()
        };
        // color trailing context if any as well as the closing quote character
        let post_idx = literal.pre() + literal.len();
        let (ctx2, post) = if post_idx > end {
            (
                context.apply_to(&data[end..post_idx]).to_string(),
                cutoff.apply_to(&data[post_idx..]).to_string(),
            )
        } else if end < data.len() {
            (String::new(), cutoff.apply_to(&data[end..]).to_string())
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
            extracted: r#"" if a layer is provided a identiacla \"input\" and \"output\", it will only be supplied an""#,
            trimmed: r#" if a layer is provided a identiacla \"input\" and \"output\", it will only be supplied an"#,
            extracted_span: Span {
                start: LineColumn {
                    line: 3usize,
                    column: 7usize - GAENSEFUESSCHEN,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 96usize + GAENSEFUESSCHEN,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 3usize,
                    column: 7usize,
                },
                end: LineColumn {
                    line: 3usize,
                    column: 96usize,
                },
            },
        },
        // 7
        Triplet {
            source: r#"

/// A ← αA<sup>OP</sup>x + βy
fn unicode(&self) -> bool {
    true
}

"#,
            extracted: r#"" A ← αA<sup>OP</sup>x + βy""#,
            trimmed: r#" A ← αA<sup>OP</sup>x + βy"#,
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
    #[ignore]
    fn raw_variant_7_unicode_symbols() {
        comment_variant_span_range_validation(7);
    }
}
