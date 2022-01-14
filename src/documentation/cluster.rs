//! Cluster `proc_macro2::Literal`s into `LiteralSets`

use syn::spanned::Spanned;
use syn::LitStr;
use syn::Macro;
use syn::Token;

use super::{trace, LiteralSet, TokenTree, TrimmedLiteral};
use crate::documentation::developer::extract_comments;
use crate::errors::*;
use crate::Span;

mod kw {
    syn::custom_keyword!(doc);
}

enum DocContent {
    LitStr(LitStr),
    Macro(Macro),
}
impl DocContent {
    fn span(&self) -> proc_macro2::Span {
        match self {
            Self::LitStr(inner) => inner.span(),
            Self::Macro(inner) => inner.span(),
        }
    }
}

struct DocComment {
    #[allow(dead_code)]
    doc: kw::doc,
    #[allow(dead_code)]
    eq_token: Token![=],
    content: DocContent,
}

impl syn::parse::Parse for DocComment {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let doc = input.parse::<kw::doc>()?;
        let eq_token: Token![=] = input.parse()?;

        let lookahead = input.lookahead1();
        let content = if lookahead.peek(LitStr) {
            input.parse().map(DocContent::LitStr)?
        } else {
            input.parse().map(DocContent::Macro)?
        };
        Ok(Self {
            doc,
            eq_token,
            content,
        })
    }
}

/// Cluster comments together, such they appear as continuous
/// text blocks.
#[derive(Debug)]
pub struct Clusters {
    pub(super) set: Vec<LiteralSet>,
}

impl Clusters {
    /// From the given source text, extracts developer comments to `LiteralSet`s
    /// and adds them to this `Clusters`
    fn parse_comments(&mut self, source: &str, dev_comments: bool) {
        let comments = extract_comments(source);
        for comment in comments {
            self.set.push(comment);
        }
    }

    /// Sort the `LiteralSet`s in this `Cluster` by start line descending, to
    /// ensure that the comments higher up in the source file appear first to
    /// the user
    fn ensure_sorted(&mut self) {
        self.set.sort_by(|ls1, ls2| ls1.coverage.cmp(&ls2.coverage));
    }

    /// Load clusters from a `&str`. Optionally loads developer comments as
    /// well.
    pub(crate) fn load_from_str(source: &str, dev_comments: bool) -> Result<Self> {
        let mut chunk = Self {
            set: Vec::with_capacity(64),
        };

        chunk.parse_comments(source, dev_comments);
        chunk.ensure_sorted();
        Ok(chunk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_comment_parse() {
        let _ = syn::parse_str::<DocComment>(r########"doc=foo!(bar!(xxx))"########).unwrap();
        let _ = syn::parse_str::<DocComment>(r########"doc="s""########).unwrap();
        let _ = syn::parse_str::<DocComment>(r########"doc=r#"s"#"########).unwrap();
        let _ = syn::parse_str::<DocComment>(r########"doc=r##"s"##"########).unwrap();
        let _ = syn::parse_str::<DocComment>(r########"doc=r###"s"###"########).unwrap();
        let _ = syn::parse_str::<DocComment>(r########"doc=r####"s"####"########).unwrap();
    }

    #[test]
    fn create_cluster() {
        static CONTENT: &str = r#####"
mod mm_mm {

/// A
#[doc=foo!(B)]
/// C
#[doc=r##"D"##]
struct X;

}
"#####;
        let clusters = Clusters::load_from_str(CONTENT, false).unwrap();
        assert_eq!(clusters.set.len(), 1);
        dbg!(&clusters.set[0]);
    }
}
