//! Cluster `proc_macro2::Literal`s into `LiteralSets`

use syn::spanned::Spanned;
use syn::LitStr;
use syn::Macro;
use syn::Token;

use super::{trace, LiteralSet, TokenTree, TrimmedLiteral};
use crate::documentation::developer::extract_developer_comments;
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
    /// Only works if the file is processed line by line, otherwise requires a
    /// adjacency list.
    fn process_literal(&mut self, source: &str, comment: DocComment) -> Result<()> {
        let span = Span::from(comment.content.span());
        let trimmed_literal = match comment.content {
            DocContent::LitStr(_s) => TrimmedLiteral::load_from(source, span)?,
            DocContent::Macro(_) => {
                TrimmedLiteral::new_empty(source, span, crate::CommentVariant::MacroDocEqMacro)
            }
        };
        if let Some(cls) = self.set.last_mut() {
            if let Err(trimmed_literal) = cls.add_adjacent(trimmed_literal) {
                trace!(target: "documentation",
                    "appending, but failed to append: {:?} to set {:?}",
                    &trimmed_literal,
                    &cls
                );
                self.set.push(LiteralSet::from(trimmed_literal))
            } else {
                trace!("successfully appended to existing: {:?} to set", &cls);
            }
        } else {
            self.set.push(LiteralSet::from(trimmed_literal));
        }
        Ok(())
    }

    /// Helper function to parse a stream and associate the found literals.
    fn parse_token_tree(&mut self, source: &str, stream: proc_macro2::TokenStream) -> Result<()> {
        let mut iter = stream.into_iter();
        while let Some(tree) = iter.next() {
            match tree {
                TokenTree::Group(group) => {
                    if let Ok(comment) = syn::parse2::<DocComment>(group.stream()) {
                        if let Err(e) = self.process_literal(source, comment) {
                            log::error!(
                                "BUG: Failed to guarantee literal content/span integrity: {}",
                                e
                            );
                            continue;
                        }
                    } else {
                        self.parse_token_tree(source, group.stream())?;
                    }
                }
                _ => {}
            };
        }
        Ok(())
    }

    /// From the given source text, extracts developer comments to `LiteralSet`s
    /// and adds them to this `Clusters`
    fn parse_developer_comments(&mut self, source: &str) {
        let developer_comments = extract_developer_comments(source);
        self.set.extend(developer_comments.into_iter());
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
        let stream = syn::parse_str::<proc_macro2::TokenStream>(source)
            .wrap_err_with(|| eyre!("Failed to parse content to stream"))?;
        chunk.parse_token_tree(source, stream)?;
        if dev_comments {
            chunk.parse_developer_comments(source);
        }
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
