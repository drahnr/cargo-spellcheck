use crate::documentation::CheckableChunk;
use crate::{Range, Span, LineColumn};
use std::fmt;
use std::convert::TryFrom;
use anyhow::{Error,Result,anyhow};
use lazy_static::lazy_static;
use regex::Regex;


#[derive(Clone, Copy, Hash, PartialEq, Eq)]
/// A ref to a trimmed literal.
pub struct TrimmedLiteralRef<'l> {
    reference: &'l TrimmedLiteral,
}

impl<'l> std::ops::Deref for TrimmedLiteralRef<'l> {
    type Target = proc_macro2::Literal;
    fn deref(&self) -> &Self::Target {
        &self.reference.literal
    }
}

impl<'l> From<&'l TrimmedLiteral> for TrimmedLiteralRef<'l> {
    fn from(anno: &'l TrimmedLiteral) -> Self {
        Self { reference: anno }
    }
}

impl<'l> TrimmedLiteralRef<'l> {
    pub fn pre(&self) -> usize {
        self.reference.pre
    }
    pub fn post(&self) -> usize {
        self.reference.pre
    }
    pub fn as_str(&self) -> &str {
        self.reference.as_str()
    }
    pub fn len(&self) -> usize {
        self.reference.len
    }
    pub fn as_ref(&self) -> &TrimmedLiteral {
        self.reference
    }

    #[allow(unused)]
    pub(crate) fn display(&self, highlight: Range) -> TrimmedLiteralDisplay {
        self.reference.display(highlight)
    }
}

impl<'l> fmt::Debug for TrimmedLiteralRef<'l> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.reference.fmt(formatter)
    }
}

/// A literal with meta info where the first and list whitespace may be found.
#[derive(Clone)]
pub struct TrimmedLiteral {
    /// The literal which this annotates to.
    pub literal: proc_macro2::Literal,
    /// the complete rendered string including post and pre.
    pub rendered: String,
    /// Whitespace prefix len + 1
    pub pre: usize,
    /// Whitespace postfix len + 1
    pub post: usize,
    /// Length of rendered **minus** `pre` and `post`.
    /// If the literal is all empty, `pre` and `post` become `0`, and `len` covers the full length of `rendered`.
    pub len: usize,
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
        if self.literal.span().start() != other.literal.span().start() {
            return false;
        }
        if self.literal.span().end() != other.literal.span().end() {
            return false;
        }

        true
    }
}

impl std::cmp::Eq for TrimmedLiteral {}

impl std::hash::Hash for TrimmedLiteral {
    fn hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        self.rendered.hash(hasher);
        self.pre.hash(hasher);
        self.post.hash(hasher);
        self.len.hash(hasher);
        Span::from(self.literal.span()).hash(hasher);
    }
}


impl TryFrom<proc_macro2::Literal> for TrimmedLiteral {
    type Error = anyhow::Error;
    fn try_from(literal: proc_macro2::Literal) -> Result<Self> {
        let rendered = dbg!(literal.to_string());

        lazy_static::lazy_static!{
            static ref PREFIX_ERASER: Regex = Regex::new(r##"^((?:r#+)?")"##).unwrap();
            static ref SUFFIX_ERASER: Regex = Regex::new(r##"("#*)$"##).unwrap();
        };

        let pre = if let Some(captures) = PREFIX_ERASER.captures(rendered.as_str()) {
            if  let Some(prefix) = captures.get(1) {
                prefix.as_str().len()
            } else {
                return Err(anyhow!("Unknown prefix of literal"))
            }
        } else {
            return Err(anyhow!("Missing prefix of literal"))
        };
        let post = if let Some(captures) = SUFFIX_ERASER.captures(rendered.as_str()) {
            // capture indices are 1 based, 0 is the full string
            if let  Some(suffix) = captures.get(captures.len() - 1) {
                suffix.as_str().len()
            } else {
                return Err(anyhow!("Unknown prefix of literal"))
            }
        } else {
            return Err(anyhow!("Missing prefix of literal"))
        };
        let scrap = |c: &'_ char| -> bool { c.is_whitespace() };

        let (len, pre, post) = match rendered.len() {
            len if len >= pre + post => (len - pre - post, pre, post),
            len => return Err(anyhow!("Prefix and suffix overlap, which is impossible")),
        };

        Ok(Self {
            len,
            literal,
            rendered,
            pre,
            post,
        })
    }
}

impl std::ops::Deref for TrimmedLiteral {
    type Target = proc_macro2::Literal;
    fn deref(&self) -> &Self::Target {
        &self.literal
    }
}

impl TrimmedLiteral {
    pub fn as_str(&self) -> &str {
        &self.rendered.as_str()[self.pre..(self.pre + self.len)]
    }

    pub fn as_untrimmed_str(&self) -> &str {
        &self.rendered.as_str()
    }

    pub fn len(&self) -> usize {
        self.len
    }

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
            cutoff.apply_to(&self.rendered.as_str()[0..self.pre]),
            pick.apply_to(&self.rendered.as_str()[self.pre..(self.pre + self.len)]),
            cutoff.apply_to(&self.rendered.as_str()[(self.pre + self.len)..]),
        )
    }
}






/// A display style wrapper for a trimmed literal.
///
/// Allows better display of coverage results without code duplication.
///
/// Consists of literal reference and a relative range to the start of the literal.
#[derive(Debug, Clone)]
pub struct TrimmedLiteralDisplay<'a>(pub TrimmedLiteralRef<'a>, pub Range);

impl<'a, R> From<(R, Range)> for TrimmedLiteralDisplay<'a>
where
    R: Into<TrimmedLiteralRef<'a>>,
{
    fn from(tuple: (R, Range)) -> Self {
        let tuple0 = tuple.0.into();
        Self(tuple0, tuple.1)
    }
}

impl<'a> Into<(TrimmedLiteralRef<'a>, Range)> for TrimmedLiteralDisplay<'a> {
    fn into(self) -> (TrimmedLiteralRef<'a>, Range) {
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
        let data = literal.as_ref().rendered.as_str();

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
            .map(|literal| TrimmedLiteral::try_from(literal).expect("Literals must be convertable to trimmed literals"))
            .collect()
	}


	const PREFIX_RAW_LEN: usize = 3;
	const SUFFIX_RAW_LEN: usize = 2;
	const GAENSEFUESSCHEN: usize = 1;

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
                    column: 0, // @todo why????
                },
                end: LineColumn {
                    line: 2usize,
                    column: 11usize, // @todo why???
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 2usize,
                    column: 3usize, // the expected start of our data
                },
                end: LineColumn {
                    line: 2usize,
                    column: 11usize,
                },
            },
        },
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
                    column: 0usize + 9 - GAENSEFUESSCHEN,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 7usize + 9 + GAENSEFUESSCHEN,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 2usize,
                    column: 0usize + 9,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 7usize + 9,
                },
            },
        },
        Triplet {
            source: r##"
#[doc = r#"Three Doc"#]
struct Three;
"##,
            extracted: r###"r#"Three Doc"#"###,
            trimmed: "Three Doc",
            extracted_span: Span {
                start: LineColumn {
                    line: 2usize,
                    column: 0usize + 11 - PREFIX_RAW_LEN,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 9usize + 11 + SUFFIX_RAW_LEN,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 2usize,
                    column: 0usize + 11,
                },
                end: LineColumn {
                    line: 2usize,
                    column: 9usize + 11,
                },
            },
        },
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
    ];

    #[test]
    fn raw_variants_inspection() {

        for triplet in TEST_DATA {
            let literals = annotated_literals(triplet.source);
            assert_eq!(literals.len(), 1);
            let trimmed_literal = literals.first().expect("Must contain exactly one literal");
            // use the raw `proc_macro2::Literal`
            let literal = &trimmed_literal.literal;

            assert_eq!(Span::from(dbg!(literal).span()), triplet.extracted_span);
            assert_eq!(literal.to_string(), triplet.extracted.to_string());
			assert_eq!(trimmed_literal.as_str(), triplet.trimmed);
			// @todo trimmed_span must be asserted too
        }
    }
}
