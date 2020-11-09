//! Representation of multiple documents.
//!
//! So to speak documentation of project as whole.
//!
//! A `literal` is a token provided by `proc_macro2`, which is then
//! converted by means of `TrimmedLiteral` using `Cluster`ing
//! into a `CheckableChunk` (mostly named just `chunk`).
//!
//! `CheckableChunk`s can consist of multiple fragments, where
//! each fragment can span multiple lines, yet each fragment
//! is covering a consecutive `Span` in the origin content.
//! Each fragment also has a direct mapping to the `CheckableChunk` internal
//! string representation.

use super::*;

use anyhow::{anyhow, Result};
use indexmap::IndexMap;
use log::trace;
pub use proc_macro2::LineColumn;
use proc_macro2::{Spacing, TokenTree};
use rayon::prelude::*;
use std::convert::{TryFrom, TryInto};
use std::path::PathBuf;

/// Range based on `usize`, simplification.
pub type Range = core::ops::Range<usize>;

mod chunk;
mod cluster;
mod literal;
pub(crate) mod literalset;
mod markdown;

pub use chunk::*;
pub use cluster::*;
pub use literal::*;
pub use literalset::*;
pub use markdown::*;
/// Collection of all the documentation entries across the project
#[derive(Debug, Clone)]
pub struct Documentation {
    /// Mapping of a path to documentation literals
    index: IndexMap<ContentOrigin, Vec<CheckableChunk>>,
}

impl Documentation {
    /// Create a new and empty doc.
    pub fn new() -> Self {
        Self {
            index: IndexMap::with_capacity(64),
        }
    }

    /// Check if the document contains any checkable items.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Borrowing iterator across content origins and associated sets of chunks.
    #[inline(always)]
    pub fn iter(&self) -> impl Iterator<Item = (&ContentOrigin, &Vec<CheckableChunk>)> {
        self.index.iter()
    }

    /// Borrowing iterator across content origins and associated sets of chunks.
    pub fn par_iter(&self) -> impl ParallelIterator<Item = (&ContentOrigin, &Vec<CheckableChunk>)> {
        self.index.par_iter()
    }

    /// Consuming iterator across content origins and associated sets of chunks.
    #[inline(always)]
    pub fn into_iter(self) -> impl Iterator<Item = (ContentOrigin, Vec<CheckableChunk>)> {
        self.index.into_iter()
    }

    /// Consuming iterator across content origins and associated sets of chunks.
    pub fn into_par_iter(
        self,
    ) -> impl ParallelIterator<Item = (ContentOrigin, Vec<CheckableChunk>)> {
        self.index.into_par_iter()
    }

    /// Join `self` with another doc to form a new one.
    pub fn join(&mut self, other: Documentation) -> &mut Self {
        other
            .into_iter()
            .for_each(|(origin, chunks): (_, Vec<CheckableChunk>)| {
                let _ = self.add_inner(origin, chunks);
            });
        self
    }

    /// Extend `self` by joining in other `Documentation`s.
    pub fn extend<I, J>(&mut self, docs: I)
    where
        I: IntoIterator<Item = Documentation, IntoIter = J>,
        J: Iterator<Item = Documentation>,
    {
        docs.into_iter().for_each(|other| {
            self.join(other);
        });
    }

    /// Adds a set of `CheckableChunk`s to the documentation to be checked.
    fn add_inner(&mut self, origin: ContentOrigin, mut chunks: Vec<CheckableChunk>) {
        self.index
            .entry(origin)
            .and_modify(|acc: &mut Vec<CheckableChunk>| {
                acc.append(&mut chunks);
            })
            .or_insert_with(|| chunks);
        // Ok(()) TODO make this failable
    }

    /// Adds a rust content str to the documentation.
    pub fn add_rust(&mut self, origin: ContentOrigin, content: &str) -> Result<()> {
        let cluster = Clusters::try_from(content)?;

        let chunks = Vec::<CheckableChunk>::from(cluster);
        self.add_inner(origin, chunks);
        Ok(())
    }

    /// Adds a common mark content str to the documentation.
    pub fn add_commonmark(&mut self, origin: ContentOrigin, content: &str) -> Result<()> {
        // extract the full content span and range
        let start = LineColumn { line: 1, column: 0 };
        let end = content
            .lines()
            .enumerate()
            .last()
            .map(|(idx, linecontent)| (idx + 1, linecontent))
            .map(|(linenumber, linecontent)| LineColumn {
                line: linenumber,
                column: linecontent.chars().count().saturating_sub(1),
            })
            .ok_or_else(|| anyhow!("Common mark / markdown file does not contain a single line"))?;

        let span = Span { start, end };
        let source_mapping = indexmap::indexmap! {
            0..content.chars().count() => span
        };
        self.add_inner(
            origin,
            vec![CheckableChunk::from_str(
                content,
                source_mapping,
                CommentVariant::CommonMark,
            )],
        );
        Ok(())
    }

    /// Obtain the set of chunks for a particular origin.
    #[inline(always)]
    pub fn get(&self, origin: &ContentOrigin) -> Option<&[CheckableChunk]> {
        self.index.get(origin).map(AsRef::as_ref)
    }

    /// Count the number of origins.
    #[inline(always)]
    pub fn entry_count(&self) -> usize {
        self.index.len()
    }
}

/// only a shortcut to avoid duplicate code
impl From<(ContentOrigin, &str)> for Documentation {
    fn from((origin, content): (ContentOrigin, &str)) -> Self {
        let mut docs = Documentation::new();

        match &origin {
            ContentOrigin::RustDocTest(_path, span) => {
                if let Ok(excerpt) =
                    crate::util::load_span_from(&mut content.as_bytes(), span.clone())
                {
                    docs.add_rust(origin.clone(), excerpt.as_str())
                } else {
                    // TODO
                    Ok(())
                }
            }
            ContentOrigin::RustSourceFile(_path) => docs.add_rust(origin, content),
            ContentOrigin::CommonMarkFile(_path) => docs.add_commonmark(origin, content),
            #[cfg(test)]
            ContentOrigin::TestEntityRust => docs.add_rust(origin, content),
            #[cfg(test)]
            ContentOrigin::TestEntityCommonMark => docs.add_commonmark(origin, content),
        }
        .unwrap_or_else(|e| warn!("BUG! << failed to load yada >> {}", e));
        docs
    }
}

#[cfg(test)]
mod tests;