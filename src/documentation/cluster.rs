//! Cluster `proc_macro2::Literal`s into `LiteralSets`

use super::{trace, LiteralSet, Spacing, TokenTree, TrimmedLiteral, TryInto};
use crate::documentation::developer::extract_developer_comments;
use crate::documentation::Range;
use crate::errors::*;
use crate::Span;
use std::convert::TryFrom;

/// Cluster comments together, such they appear as continuous
/// text blocks.
#[derive(Debug)]
pub struct Clusters {
    pub(super) set: Vec<LiteralSet>,
}

impl Clusters {
    /// Only works if the file is processed line by line, otherwise requires a
    /// adjacency list.
    fn process_literal(&mut self, source: &str, span: Span) -> Result<()> {
        let trimmed_literal = TrimmedLiteral::try_from((source, span))?;
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
                TokenTree::Ident(ident) => {
                    // if we find an identifier
                    // which is doc
                    if ident != "doc" {
                        continue;
                    }

                    // this assures the sequence is as anticipated
                    let op = iter.next();
                    if op.is_none() {
                        continue;
                    }
                    let op = op.unwrap();
                    if let TokenTree::Punct(punct) = op {
                        if punct.as_char() != '=' {
                            continue;
                        }
                        if punct.spacing() != Spacing::Alone {
                            continue;
                        }
                    } else {
                        continue;
                    }

                    let comment = if let Some(comment) = iter.next() {
                        comment
                    } else {
                        continue
                    };

                    match comment {
                        TokenTree::Literal(literal) => {
                            let span = Span::from(literal.span());
                            trace!(target: "documentation",
                                "Found doc literal at {:?}: {:?}",
                                <Span as TryInto<Range>>::try_into(span.clone()),
                                literal
                            );

                            // let rendered = literal.to_string();
                            // produces pretty unusable garabage, since it modifies the content of `///`
                            // comments which could contain " which will be escaped
                            // and therefor cause the `span()` to yield something that does
                            // not align with the rendered literal at all and there are too
                            // many pitfalls to sanitize all cases, so reading given span
                            // from the file again, and then determining its type is way safer.

                            if let Err(e) = self.process_literal (source, span) {
                                log::error!(
                                    "BUG: Failed to guarantee literal content/span integrity: {}",
                                    e
                                );
                                continue;
                            }
                        },
                        _x => {
                            dbg!(_x);
                            continue;
                        },
                    }
                }
                TokenTree::Group(group) => {
                    self.parse_token_tree(source, group.stream())?;
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
        for comment in developer_comments {
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
