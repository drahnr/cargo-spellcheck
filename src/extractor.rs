//! Executes the actual path traversal and creating a token stream.
//!
//! Whatever.

use super::*;
use std::fs;

use indexmap::IndexMap;
use log::{debug, info, trace, warn};
use proc_macro2::{Spacing, TokenTree};

use std::path::{Path, PathBuf};

/// Complete set of documentation for a set of files.
#[doc = "check"]
#[derive(Debug, Clone)]
pub struct Documentation {
    /// Mapping of a path to documentation literals
    index: IndexMap<String, Vec<proc_macro2::Literal>>,
}

impl Documentation {
    pub fn new() -> Self {
        Self {
            index: IndexMap::with_capacity(64),
        }
    }

    pub fn join(&mut self, other: Documentation) -> &mut Self {
        other.index.into_iter().for_each(|(path, literals)| {
            self.index
                .entry(path)
                .and_modify(|acc| acc.extend_from_slice(literals.as_slice()))
                .or_insert_with(|| literals);
        });
        self
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Vec<proc_macro2::Literal>)> {
        self.index.iter()
    }

    pub fn into_iter(self) -> impl Iterator<Item = (String, Vec<proc_macro2::Literal>)> {
        self.index.into_iter()
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
}

impl<S> From<(S, proc_macro2::TokenStream)> for Documentation
where
    S: AsRef<str>,
{
    fn from(tup: (S, proc_macro2::TokenStream)) -> Self {
        let (path, stream) = tup;
        let path = path.as_ref().to_owned();

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
                        documentation
                            .index
                            .entry(path.clone())
                            .or_insert_with(|| Vec::new())
                            .push(literal);
                    } else {
                        continue;
                    }
                }
                TokenTree::Group(group) => {
                    let _ = documentation.join(Documentation::from((&path, group.stream())));
                }
                _ => {}
            };
        }
        documentation
    }
}

/// TODO do this incremental, first parse the firstt file
/// and extract all mod declarations and filter the subdirs and files based on
/// the modules names. That way stale files can be avoided.
pub(crate) fn traverse(path: &Path) -> anyhow::Result<Vec<Documentation>> {
    let sources = walkdir::WalkDir::new(path)
        .max_depth(45)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry: &walkdir::DirEntry| -> bool { entry.file_type().is_file() })
        .filter_map(|entry| Some(entry.path().to_str()?.to_owned()))
        .filter(|path| path.ends_with(".rs"))
        .collect::<Vec<String>>();

    let documentation = sources
        .iter()
        .filter_map(|path: &String| -> Option<Documentation> {
            fs::read_to_string(path)
                .ok()
                .and_then(|content: String| syn::parse_str(&content).ok())
                .map(|stream| Documentation::from((path, stream)))
        })
        .filter(|documentation| !documentation.is_empty())
        .collect();
    Ok(documentation)
}

pub(crate) fn run(mode: Mode, paths: Vec<PathBuf>, recurse: bool) -> anyhow::Result<()> {
    // TODO honour recurse flag

    let docs: Vec<Documentation> = if recurse {
        trace!("Recursive");
        paths
            .iter()
            .try_fold::<Vec<Documentation>, _, anyhow::Result<Vec<Documentation>>>(
                Vec::with_capacity(paths.len()),
                |mut acc, path| {
                    let content = fs::read_to_string(&path)?;
                    let stream = syn::parse_str(&content)?;
                    let path: String = path.to_str().unwrap().to_owned();
                    acc.push(Documentation::from((path, stream)));
                    Ok(acc)
                },
            )?
    } else {
        trace!("Single file");
        paths
            .iter()
            .try_fold::<Vec<Documentation>, _, anyhow::Result<Vec<Documentation>>>(
                Vec::with_capacity(paths.len()),
                |mut acc, path| {
                    let mut doc = traverse(path)?;
                    acc.append(&mut doc);
                    Ok(acc)
                },
            )?
    };

    let combined = Documentation::combine(docs);
    let suggestions = crate::checker::check(&combined)?;

    match mode {
        Mode::Fix => unimplemented!("Unsupervised fixing is not implemented just yet"),
        Mode::Check => {
            for suggestion in suggestions {
                eprintln!("{}", suggestion);
            }
        }
        Mode::Interactive => unimplemented!("Interactive pick & apply is not implemented just yet"),
    }

    Ok(())
}
