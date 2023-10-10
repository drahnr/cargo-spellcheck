//! # Doc Chunks
//!
//! `Documentation` is a representation of one or multiple documents.
//!
//! A `literal` is a token provided by `proc_macro2` or `ra_ap_syntax` crate, which is then converted by
//! means of `TrimmedLiteral` using `Cluster`ing into a `CheckableChunk` (mostly
//! named just `chunk`).
//!
//! `CheckableChunk`s can consist of multiple fragments, where each fragment can
//! span multiple lines, yet each fragment is covering a consecutive `Span` in
//! the origin content. Each fragment also has a direct mapping to the
//! `CheckableChunk` internal string representation.
//!
//! And `Documentation` holds one or many `CheckableChunks` per file path.

#![deny(unused_crate_dependencies)]

// contains test helpers
pub mod span;
pub mod testcase;
pub use self::span::Span;
pub use proc_macro2::LineColumn;

pub mod util;
use self::util::{load_span_from, sub_char_range};

use indexmap::IndexMap;
use proc_macro2::TokenTree;
use rayon::prelude::*;
use serde::Deserialize;
use std::path::PathBuf;
use toml::Spanned;

/// Range based on `usize`, simplification.
pub type Range = core::ops::Range<usize>;

/// Apply an offset to `start` and `end` members, equaling a shift of the range.
pub fn apply_offset(range: &mut Range, offset: usize) {
    range.start = range.start.saturating_add(offset);
    range.end = range.end.saturating_add(offset);
}

pub mod chunk;
pub mod cluster;
mod developer;
pub mod errors;
pub mod literal;
pub mod literalset;
pub mod markdown;

pub use chunk::*;
pub use cluster::*;
pub use errors::*;
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

    /// Check if a particular key is contained.
    pub fn contains_key(&self, key: &ContentOrigin) -> bool {
        self.index.contains_key(key)
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
    pub fn into_par_iter(
        self,
    ) -> impl ParallelIterator<Item = (ContentOrigin, Vec<CheckableChunk>)> {
        self.index.into_par_iter()
    }

    /// Extend `self` by joining in other `Documentation`s.
    pub fn extend<I, J>(&mut self, other: I)
    where
        I: IntoIterator<Item = (ContentOrigin, Vec<CheckableChunk>), IntoIter = J>,
        J: Iterator<Item = (ContentOrigin, Vec<CheckableChunk>)>,
    {
        other
            .into_iter()
            .for_each(|(origin, chunks): (_, Vec<CheckableChunk>)| {
                let _ = self.add_inner(origin, chunks);
            });
    }

    /// Adds a set of `CheckableChunk`s to the documentation to be checked.
    pub fn add_inner(&mut self, origin: ContentOrigin, mut chunks: Vec<CheckableChunk>) {
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
        doc_comments: bool,
        dev_comments: bool,
    ) -> Result<()> {
        let cluster = Clusters::load_from_str(content, doc_comments, dev_comments)?;

        let chunks = Vec::<CheckableChunk>::from(cluster);
        self.add_inner(origin, chunks);
        Ok(())
    }

    /// Adds a content string to the documentation sourced from the
    /// `description` field in a `Cargo.toml` manifest.
    pub fn add_cargo_manifest_description(
        &mut self,
        path: PathBuf,
        manifest_content: &str,
    ) -> Result<()> {
        fn extract_range_of_description(manifest_content: &str) -> Result<Range> {
            #[derive(Deserialize, Debug)]
            struct Manifest {
                package: Spanned<Package>,
            }

            #[derive(Deserialize, Debug)]
            struct Package {
                description: Spanned<String>,
            }

            let value: Manifest = toml::from_str(manifest_content)?;
            let d = value.package.into_inner().description;
            let range = d.start()..d.end();
            Ok(range)
        }

        let mut range = extract_range_of_description(&manifest_content)?;
        let description = sub_char_range(&manifest_content, range.clone());

        // Attention: `description` does include `\"\"\"` as well as `\\\n`, the latter is not a big issue,
        // but the trailing start and end delimiters are.
        // TODO: split into multiple on `\\\n` and create multiple range/span mappings.
        let description = if range.len() > 6 {
            if description.starts_with("\"\"\"") {
                range.start += 3;
                range.end -= 3;
                assert!(!range.is_empty());
            }
            dbg!(&description[3..(description.len()) - 3])
        } else {
            description
        };

        fn convert_range_to_span(content: &str, range: Range) -> Option<Span> {
            let mut line = 0_usize;
            let mut column = 0_usize;
            let mut prev = '\n';
            let mut start = None;
            for (offset, c) in content.chars().enumerate() {
                if prev == '\n' {
                    column = 0;
                    line += 1;
                }
                prev = c;

                if offset == range.start {
                    start = Some(LineColumn { line, column });
                    continue;
                }
                // take care of inclusivity
                if offset + 1 == range.end {
                    let end = LineColumn { line, column };
                    return Some(Span {
                        start: start.unwrap(),
                        end,
                    });
                }
                column += 1;
            }
            None
        }

        let span = convert_range_to_span(manifest_content, range.clone()).expect(
            "Description is part of the manifest since it was parsed from the same source. qed",
        );
        let origin = ContentOrigin::CargoManifestDescription(path);
        let source_mapping = dbg!(indexmap::indexmap! {
            range => span
        });
        self.add_inner(
            origin,
            vec![CheckableChunk::from_str(
                description,
                source_mapping,
                CommentVariant::TomlEntry,
            )],
        );
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
            .ok_or_else(|| {
                Error::Span(
                    "Common mark / markdown file does not contain a single line".to_string(),
                )
            })?;

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
    pub fn load_from_str(
        origin: ContentOrigin,
        content: &str,
        doc_comments: bool,
        dev_comments: bool,
    ) -> Self {
        let mut docs = Documentation::new();

        match origin.clone() {
            ContentOrigin::RustDocTest(_path, span) => {
                if let Ok(excerpt) = load_span_from(&mut content.as_bytes(), span.clone()) {
                    docs.add_rust(origin.clone(), excerpt.as_str(), doc_comments, dev_comments)
                } else {
                    // TODO
                    Ok(())
                }
            }
            origin @ ContentOrigin::RustSourceFile(_) => {
                docs.add_rust(origin, content, doc_comments, dev_comments)
            }
            ContentOrigin::CargoManifestDescription(path) => {
                docs.add_cargo_manifest_description(path, content)
            }
            origin @ ContentOrigin::CommonMarkFile(_) => docs.add_commonmark(origin, content),
            origin @ ContentOrigin::TestEntityRust => {
                docs.add_rust(origin, content, doc_comments, dev_comments)
            }
            origin @ ContentOrigin::TestEntityCommonMark => docs.add_commonmark(origin, content),
        }
        .unwrap_or_else(move |e| {
            log::warn!(
                "BUG: Failed to load content from {} (dev_comments={:?}): {:?}",
                origin,
                dev_comments,
                e
            );
        });
        docs
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }
}

impl IntoIterator for Documentation {
    type Item = (ContentOrigin, Vec<CheckableChunk>);
    type IntoIter = indexmap::map::IntoIter<ContentOrigin, Vec<CheckableChunk>>;

    fn into_iter(self) -> Self::IntoIter {
        self.index.into_iter()
    }
}
