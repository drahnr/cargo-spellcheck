//! Chunk definition for what is going to be processed by the checkers

use super::*;
use anyhow::{anyhow, Error, Result};
use indexmap::IndexMap;

use crate::{Range, Span};
use crate::documentation::PlainOverlay;

/// Definition of the source of a checkable chunk
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum ContentOrigin {
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


    pub fn erase_markdown(&self) -> PlainOverlay {
        PlainOverlay::erase_markdown(self)
    }



    /// Convert a range of the linear trimmed (but no other processing) string representation to a set of
    /// literal references and spans for the file the chunk resides in.
    // @todo check if this still needed, or if we just need range mappings
    pub fn linear_range_to_spans(
        &self,
        range: Range,
    ) -> Vec<Span> {
        unimplemented!("Should return only a Vec<Span>, we don't care about literals anymore")
        // find_coverage(&self.literals, &range)
        //     .map(|(literals, start, end)| {
        //         assert!(!literals.is_empty());
        //         trace!("coverage: {:?} -> end {:?}", &range, end);
        //         let n = literals.len();
        //         if n > 1 {
        //             let mut iter = literals.into_iter();
        //             let first: &'a _ = iter.next().unwrap();

        //             // calculate how many lines it spans
        //             let mut acc = Vec::with_capacity(n);
        //             // first literal to its end
        //             if first.span().end() != start {
        //                 acc.push((
        //                     first,
        //                     Span {
        //                         start,
        //                         end: first.span().end(),
        //                     },
        //                 ));
        //             }

        //             // take all in between the first and the last completely

        //             for literal in iter.clone().take(n - 2) {
        //                 let span = Span {
        //                     start: literal.span().start(),
        //                     end: literal.span().end(),
        //                 };
        //                 if span.start != span.end {
        //                     acc.push((literal, span));
        //                 }
        //             }
        //             // add the last from the beginning to the computed end
        //             let last: &'a _ = iter.skip(n - 2).next().unwrap();
        //             if last.span().start() != end {
        //                 acc.push((
        //                     last,
        //                     Span {
        //                         start: last.span().start(),
        //                         end,
        //                     },
        //                 ));
        //             }
        //             return acc;
        //         } else {
        //             assert_eq!(n, 1);
        //             return vec![(literals.first().unwrap(), Span { start, end })];
        //         }
        //     })
        //     .unwrap_or_else(|| Vec::new())
    }

    pub fn as_str(&self) -> &str {
        self.content.as_str()
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
