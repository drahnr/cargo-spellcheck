//! Executes the actual path traversal and creating a token stream.
//!
//! Whatever.

use super::checker::Span;
use super::*;

use std::fs;

use indexmap::IndexMap;
use log::{debug, info, trace, warn};
use proc_macro2::{Spacing, TokenTree};

pub use proc_macro2::LineColumn;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct ConsecutiveLiteralSet {
    /// consecutive set of literals mapped by line number
    literals: Vec<proc_macro2::Literal>,
    /// lines spanned (start, end)
    pub coverage: (usize, usize),
}

impl ConsecutiveLiteralSet {
    /// Initiate a new set based on the first literal
    pub fn from(literal: proc_macro2::Literal) -> Self {
        Self {
            coverage: (literal.span().start().line, literal.span().end().line),
            literals: vec![literal],
        }
    }

    /// Add a literal to a literal set, if the previous lines literal already exists.
    ///
    /// Returns literl within the Err variant if not adjacent
    pub fn add_adjacent(
        &mut self,
        literal: proc_macro2::Literal,
    ) -> Result<(), proc_macro2::Literal> {
        let previous_line = literal.span().end().line;
        if previous_line == self.coverage.0 + 1 {
            let _ = self.literals.insert(previous_line, literal);
            return Ok(());
        }

        let next_line = literal.span().start().line;
        if next_line + 1 == self.coverage.1 {
            let _ = self.literals.insert(next_line, literal);
            return Ok(());
        }

        return Err(literal);
    }

    fn extract<'a>(
        it: &mut impl Iterator<Item = &'a proc_macro2::Literal>,
        mut offset: usize,
    ) -> Option<(&'a proc_macro2::Literal, LineColumn, usize)> {
        for literal in it {
            let len = literal.to_string().len();
            if offset > len {
                offset -= len;
                continue;
            }
            if literal.span().end().column < offset {
                break;
            }
            if literal.span().start().column > offset {
                break;
            }
            return Some((
                literal,
                LineColumn {
                    line: literal.span().start().line,
                    column: offset,
                },
                offset,
            ));
        }
        None
    }

    /// Convert a linear offset to a set of offsets with literal references and spans within that literal.
    pub fn linear_coverage_to_span<'a>(
        &'a self,
        offset: usize,
        length: usize,
    ) -> Option<(&'a proc_macro2::Literal, Span)> {
        let mut x = self.literals.iter();

        if let Some((start_literal, start, mut offset)) = Self::extract(&mut x, offset) {
            offset += length;
            if let Some((_end_literal, end, _offset)) = Self::extract(&mut x, offset) {
                // if start_literal.span() != end_literal.span() {
                //     warn!("Need multiline literal coverage support #TODO");
                // }
                let span = Span {
                    start,
                    end,
                };
                return Some((start_literal, span))
            }
        }
        None
    }

    pub fn literals<'x>(&'x self) -> Vec<&'x proc_macro2::Literal> {
        self.literals.iter().by_ref().collect()
    }
}

use std::fmt;

impl<'s> fmt::Display for ConsecutiveLiteralSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for literal in self.literals.iter() {
            literal.fmt(formatter)?;
            formatter.write_str("\n")?;
        }
        Ok(())
    }
}

/// Complete set of documentation for a set of files.
#[doc = "check"]
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
