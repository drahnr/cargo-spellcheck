//! Executes the actual path traversal and creating a token stream.
//!
//! Whatever.

use super::*;
use std::fs;

use anyhow::anyhow;
use proc_macro2::{TokenStream, TokenTree};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::path::{Path, PathBuf};

/// Complete set of documentation in a set of paths helpz.
pub struct Documentation {
    index: HashMap<String, Vec<proc_macro2::Literal>>,
}

impl Documentation {
    pub fn new() -> Self {
        Self {
            index: HashMap::with_capacity(64),
        }
    }

    pub fn join(&mut self, other: Documentation) -> &mut Self {
        other.index.into_iter().for_each(|(path, literals)| {
            self.index
                .entry(path)
                .or_insert_with(|| Vec::with_capacity(literals.len()))
                .extend_from_slice(literals.as_slice());
        });
        self
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
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
        // state tracker
        let mut is_doc = false;
        for tree in stream {
            match tree {
                TokenTree::Ident(ident) => {
                    is_doc = ident == "doc";
                }
                TokenTree::Group(group) => {
                    // XXX recursive call
                    let _ = documentation.join(Documentation::from((&path, group.stream())));
                }
                TokenTree::Literal(literal) => {
                    if is_doc {
                        documentation
                            .index
                            .entry(path.clone())
                            .or_insert_with(|| Vec::new())
                            .push(literal);
                    }
                }
                _ => {}
            };
        }
        documentation
    }
}

pub(crate) fn traverse(path: &Path) -> anyhow::Result<Vec<Documentation>> {
    let source_files = walkdir::WalkDir::new(path)
        .max_depth(45)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry: &walkdir::DirEntry| -> bool { entry.file_type().is_file() })
        .filter_map(|entry| Some(entry.path().to_str()?.to_owned()))
        .filter(|path| path.ends_with(".rs"))
        .collect::<Vec<String>>();

    let documentation = source_files
        .iter()
        .filter_map(|path: &String| -> Option<Documentation> {
            fs::read_to_string(path)
                .map(|content: String| syn::parse_str(&content).ok())
                .ok().flatten()
                .map(|stream| Documentation::from((path, stream)))
        })
        .filter(|documentation| !documentation.is_empty())
		.collect();
	Ok(documentation)
}

pub(crate) fn run(mode: Mode, paths: Vec<PathBuf>, recurse: bool) -> anyhow::Result<()> {
    // TODO honour recurse flag

    let docs: Vec<Documentation> = if !recurse {
        paths
            .iter()
            .try_fold::<Vec<Documentation>,_,anyhow::Result<Vec<Documentation>>>(Vec::with_capacity(64), |mut acc, path| {
				let content = fs::read_to_string(&path)?;
				let stream = syn::parse_str(&content)?;
				let path: String = path.to_str().unwrap().to_owned();
				acc.push(Documentation::from((path, stream)));
                Ok(acc)
            })?
    } else {
        paths
            .into_iter()
            .try_fold::<Vec<Documentation>,_,anyhow::Result<Vec<Documentation>>>(Vec::with_capacity(64), |mut acc, path| {
                let mut doc = traverse(&path)?;
                acc.append(&mut doc);
                Ok(acc)
            })?
    };

    // TODO do smth with docs
    Ok(())
}
