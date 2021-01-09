//! Cluster `proc_macro2::Literal`s into `LiteralSets`

use super::{trace, LiteralSet, Spacing, TokenTree, TrimmedLiteral, TryInto};
use crate::documentation::Range;
use crate::Span;
use anyhow::{anyhow, Error, Result};
use std::convert::TryFrom;

/// Cluster literals for one file
#[derive(Debug)]
pub struct Clusters {
    pub(super) set: Vec<LiteralSet>,
}

impl Clusters {
    /// Only works if the file is processed line by line, otherwise
    /// requires a adjacency list.
    fn process_literal(&mut self, source: &str, literal: proc_macro2::Literal) -> Result<()> {
        let literal = TrimmedLiteral::try_from((source, literal))?;
        if let Some(cls) = self.set.last_mut() {
            if let Err(literal) = cls.add_adjacent(literal) {
                trace!(target: "documentation",
                    "appending, but failed to append: {:?} to set {:?}",
                    &literal,
                    &cls
                );
                self.set.push(LiteralSet::from(literal))
            } else {
                trace!("successfully appended to existing: {:?} to set", &cls);
            }
        } else {
            self.set.push(LiteralSet::from(literal));
        }
        Ok(())
    }

    /// Helper function to parse a stream and associated the found literals
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

                    let comment = iter.next();
                    if comment.is_none() {
                        continue;
                    }
                    let comment = comment.unwrap();
                    if let TokenTree::Literal(literal) = comment {
                        trace!(target: "documentation",
                            "Found doc literal at {:?}: {:?}",
                            <Span as TryInto<Range>>::try_into(Span::from(literal.span())),
                            literal
                        );
                        if let Err(e) = self.process_literal(source, literal) {
                            log::error!(
                                "BUG: Failed to guarantee literal content/span integrity: {}",
                                e
                            );
                            continue;
                        }
                    } else {
                        continue;
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
}

impl TryFrom<&str> for Clusters {
    type Error = Error;
    fn try_from(source: &str) -> Result<Self> {
        let mut chunk = Self {
            set: Vec::with_capacity(64),
        };
        let stream = syn::parse_str::<proc_macro2::TokenStream>(source)
            .map_err(|e| anyhow!("Failed to parse content to stream").context(e))?;
        chunk.parse_token_tree(source, stream)?;
        Ok(chunk)
    }
}
