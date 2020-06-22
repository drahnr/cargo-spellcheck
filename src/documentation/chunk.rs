//! Chunk definition for what is going to be processed by the checkers

use super::*;
use crate::{Range, Span};
use anyhow::{anyhow, Error, Result};

use indexmap::IndexMap;

/// Definition of the source of a checkable chunk
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum ContentSource {
    CommonMarkFile(PathBuf),
    RustDocTest(PathBuf, Span), // span is just there to disambiguiate
    RustSourceFile(PathBuf),
}

/// A chunk of documentation that is supposed to be checked
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CheckableChunk {
    /// Rendered contents
    content: String,
    /// Mapping from range within `content` and `Span` referencing the location within the file
    source_mapping: IndexMap<Range, Span>,
}


impl std::hash::Hash for CheckableChunk {
    fn hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
		self.content.hash(hasher);
		// order is consistent
        self.source_mapping.iter().for_each(|t| {
			t.hash(hasher);
		});
    }
}

impl CheckableChunk {
    /// Specific to rust source code, either as part of doc test comments or file scope
    pub fn from_literalset(set: LiteralSet) -> Self {
        // @todo figure out the range to span mapping here
        Self::from_string(set.to_string(), IndexMap::<Range, Span>::new())
    }

    /// Load content from string, may contain markdown content
    pub fn from_str(content: &str, source_mapping: IndexMap<Range, Span>) -> Self {
        Self::from_string(content.to_string(), source_mapping)
    }

    pub fn from_string(content: String, source_mapping: IndexMap<Range, Span>) -> Self {
        Self {
            content,
            source_mapping,
        }
    }
}

/// Convert the clusters of one file into a source description as well
/// as well as vector of checkable chunks.
impl From<Clusters> for Vec<CheckableChunk> {
    fn from(clusters: Clusters) -> Vec<CheckableChunk> {
        clusters
            .set
            .into_iter()
            .map(|literal_set| {
				CheckableChunk::from_literalset(literal_set)
			}).collect::<Vec<_>>()
    }
}
