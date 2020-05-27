//! Representation of multiple documents.
//!
//! So to speak documentation of project as whole.

use super::*;
use crate::LiteralSet;

use indexmap::IndexMap;
use log::trace;
use proc_macro2::{Spacing, TokenTree};

pub use proc_macro2::LineColumn;
use std::path::{Path, PathBuf};

/// Collection of all the documentation entries across the project
#[derive(Debug, Clone)]
pub struct Documentation {
    /// Mapping of a path to documentation literals
    index: IndexMap<PathBuf, Vec<LiteralSet>>,
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

    pub fn iter(&self) -> impl Iterator<Item = (&PathBuf, &Vec<LiteralSet>)> {
        self.index.iter()
    }

    pub fn into_iter(self) -> impl Iterator<Item = (PathBuf, Vec<LiteralSet>)> {
        self.index.into_iter()
    }

    pub fn join(&mut self, other: Documentation) -> &mut Self {
        other
            .into_iter()
            .for_each(|(path, mut literals): (_, Vec<LiteralSet>)| {
                self.index
                    .entry(path)
                    .and_modify(|acc: &mut Vec<LiteralSet>| {
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
        let literal = TrimmedLiteral::from(literal);
        match self.index.entry(path.to_owned()) {
            indexmap::map::Entry::Occupied(occupied) => {
                let v = occupied.into_mut();
                let cls = v.last_mut().unwrap();
                if let Err(literal) = cls.add_adjacent(literal) {
                    trace!(target: "documentation",
                        "appending, but failed to append: {:?} to set {:?}",
                        &literal,
                        &cls
                    );
                    v.push(LiteralSet::from(literal))
                } else {
                    trace!("successfully appended to existing: {:?} to set", &cls);
                }
            }
            indexmap::map::Entry::Vacant(vacant) => {
                trace!(target: "documentation",
                    "nothing for {} file yet, create new literal set",
                    path.display()
                );
                vacant.insert(vec![LiteralSet::from(literal)]);
            }
        }
    }

    /// Helper function to parse a path stream and associated the found literals to `path`
    fn parse_token_tree<P: AsRef<Path>>(&mut self, path: P, stream: proc_macro2::TokenStream) {
        let path: &Path = path.as_ref();

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
                            "Found doc literal at {:?}..{:?}: {:?}",
                            literal.span().start(),
                            literal.span().end(),
                            literal
                        );
                        self.append_literal(path, literal);
                    } else {
                        continue;
                    }
                }
                TokenTree::Group(group) => {
                    self.parse_token_tree(path, group.stream());
                }
                _ => {}
            };
        }
    }
}

impl<P> From<(P, proc_macro2::TokenStream)> for Documentation
where
    P: AsRef<Path>,
{
    fn from((path, stream): (P, proc_macro2::TokenStream)) -> Self {
        let mut documentation = Documentation::new();
        documentation.parse_token_tree(path, stream);
        documentation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST: &str =
r#"/// A very good test.
///
/// Without much ado, we adhere to **King** _Ragnar_.
struct Vikings;
"#;

    const TEST_EXTRACT: &str = r#" A very good test.

 Without much ado, we adhere to **King** _Ragnar_."#;

    #[test]
    fn parse_and_construct() {
        let _ = env_logger::try_init();

        let test_path = PathBuf::from("/tmp/dummy");

        let stream = syn::parse_str(TEST).expect("Must be valid rust");
        let docs = Documentation::from((test_path.as_path(), stream));
        assert_eq!(docs.index.len(), 1);
        let v = docs.index.get(&test_path).expect("Must contain dummy path");
        assert_eq!(dbg!(v).len(), 1);
        assert_eq!(v[0].to_string(), TEST_EXTRACT.to_owned());
        let plain = v[0].erase_markdown();
        log::info!("Plain: \n {:?}", &plain);
        assert_eq!(dbg!(plain.linear_range_to_spans(1..3)).len(), 1);
        assert_eq!(v[0].linear_range_to_spans(2..4), plain.linear_range_to_spans(1..3));
    }
}
