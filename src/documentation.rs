//! Representation of multiple documents.
//!
//! So to speak documentation of project as whole.

use crate::{ConsecutiveLiteralSet,Span};
use super::*;

use std::fs;

use indexmap::IndexMap;
use log::{debug, info, trace, warn};
use proc_macro2::{Spacing, TokenTree};

pub use proc_macro2::LineColumn;
use std::path::{Path, PathBuf};

use std::fmt;

#[derive(Debug, Clone)]
pub struct Documentation {
    /// Mapping of a path to documentation literals
    index: IndexMap<PathBuf, Vec<ConsecutiveLiteralSet>>,
}

impl Documentation {
    pub fn new() -> Self {
        Self {
            index: IndexMap::with_capacity(64),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&PathBuf, &Vec<ConsecutiveLiteralSet>)> {
        self.index.iter()
    }

    pub fn into_iter(self) -> impl Iterator<Item = (PathBuf, Vec<ConsecutiveLiteralSet>)> {
        self.index.into_iter()
    }

    pub fn join(&mut self, other: Documentation) -> &mut Self {
        other
            .into_iter()
            .for_each(|(path, mut literals): (_, Vec<ConsecutiveLiteralSet>)| {
                self.index
                    .entry(path)
                    .and_modify(|acc: &mut Vec<ConsecutiveLiteralSet>| {
                        acc.append(&mut literals);
                    })
                    .or_insert_with(|| literals);
            });
        self
    }

    pub fn combine(mut docs: Vec<Documentation>) -> Documentation {
        if let Some(first) = docs.pop() {
            docs.into_iter().fold(first, |mut first, other| {
                first.join(other);
                first
            })
        } else {
            Documentation::new()
        }
    }

    /// Append a literal to the given path
    ///
    /// Only works if the file is processed line by line, otherwise
    /// requires a adjacency list.
    pub fn append_literal(&mut self, path: &Path, literal: proc_macro2::Literal) {
        let v: &mut Vec<_> = self
            .index
            .entry(path.to_owned())
            .or_insert_with(|| Vec::new());

        let literal = AnnotatedLiteral::from(literal);
        if let Some(last) = v.last_mut() {
            if let Err(literal) = last.add_adjacent(literal) {
                v.push(ConsecutiveLiteralSet::from(literal))
            }
        } else {
            v.push(ConsecutiveLiteralSet::from(literal))
        }
    }
}

impl<P> From<(P, proc_macro2::TokenStream)> for Documentation
where
    P: AsRef<Path>,
{
    fn from(tup: (P, proc_macro2::TokenStream)) -> Self {
        let (path, stream) = tup;
        let path: &Path = path.as_ref();

        let mut documentation = Documentation::new();
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
                        trace!("Found doc literal: {:?}", literal);
                        documentation.append_literal(path, literal);
                    } else {
                        continue;
                    }
                }
                TokenTree::Group(group) => {
                    let _ = documentation.join(Documentation::from((path, group.stream())));
                }
                _ => {}
            };
        }
        documentation
    }
}