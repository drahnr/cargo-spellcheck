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

use crate::errors::*;
use crate::util::load_span_from;
use indexmap::IndexMap;
use log::trace;
pub use proc_macro2::LineColumn;
use proc_macro2::{Spacing, TokenTree};
use rayon::prelude::*;
use std::convert::TryInto;
use std::path::PathBuf;

/// Range based on `usize`, simplification.
pub type Range = core::ops::Range<usize>;

/// Apply an offset to `start` and `end` members, equaling a shift of the range.
pub fn apply_offset(range: &mut Range, offset: usize) {
    range.start = range.start.saturating_add(offset);
    range.end = range.end.saturating_add(offset);
}

mod chunk;
mod cluster;
mod developer;
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
    pub fn add_rust(
        &mut self,
        origin: ContentOrigin,
        content: &str,
        dev_comments: bool,
    ) -> Result<()> {
        let cluster = Clusters::load_from_str(content, dev_comments)?;

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
            .ok_or_else(|| eyre!("Common mark / markdown file does not contain a single line"))?;

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

    /// Load a document from a single string with a defined origin.
    pub fn load_from_str(origin: ContentOrigin, content: &str, dev_comments: bool) -> Self {
        let mut docs = Documentation::new();

        match origin.clone() {
            ContentOrigin::RustDocTest(_path, span) => {
                if let Ok(excerpt) = load_span_from(&mut content.as_bytes(), span.clone()) {
                    docs.add_rust(origin.clone(), excerpt.as_str(), dev_comments)
                } else {
                    // TODO
                    Ok(())
                }
            }
            origin @ ContentOrigin::RustSourceFile(_) => {
                docs.add_rust(origin, content, dev_comments)
            }
            origin @ ContentOrigin::CommonMarkFile(_) => docs.add_commonmark(origin, content),
            #[cfg(test)]
            origin @ ContentOrigin::TestEntityRust => docs.add_rust(origin, content, dev_comments),
            #[cfg(test)]
            origin @ ContentOrigin::TestEntityCommonMark => docs.add_commonmark(origin, content),
        }
        .unwrap_or_else(move |e| {
            warn!(
                "BUG: Failed to load content from {} (dev_comments={:?}): {:?}",
                origin, dev_comments, e
            );
        });
        docs
    }
}

#[cfg(test)]
mod tests;
