//! Extract documentation by owner and developer comments by source adjacency.
//!
//! Attribute syntax, guarded documentation selection, and tolerant token traversal
//! live in separate private modules. Documentation macros are not expanded.

use proc_macro2::TokenStream;

use crate::developer::extract_developer_comments;
use crate::errors::*;
use crate::{CfgBudget, LiteralSet, Span};

mod attributes;
mod limits;
mod owner;
mod parser;
mod scope;

#[cfg(test)]
mod tests;

/// Why extraction skipped documentation or could not cover every cfg alternative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionWarningKind {
    /// Cfg reasoning exceeded its budget and checked only a bounded sample.
    CfgBudget,
    /// An unsupported attribute payload caused selected documentation variants to be skipped.
    UnsupportedAttribute,
    /// A selected documentation macro has unknown Markdown content.
    OpaqueDocumentation,
}

/// A source-spanned explanation of omitted or incompletely sampled documentation.
#[derive(Debug, Clone)]
pub struct ExtractionWarning {
    /// Classification of the extraction limitation.
    pub kind: ExtractionWarningKind,
    /// Source range of the costly predicate, unsupported attribute payload, or unexpanded macro.
    pub span: Span,
    /// Explanation of the limitation and its effect on documentation coverage.
    pub message: String,
}

/// Compatibility name for extraction warnings previously limited to cfg coverage.
pub type CfgCoverageWarning = ExtractionWarning;

/// Concrete literal sets extracted from source comments.
///
/// Documentation fragments are grouped by owner, projected into cfg alternatives,
/// and checked for known text before a literal set is emitted. Developer comments
/// are grouped by source adjacency. Selections containing unexpanded macros are
/// skipped with a warning rather than represented as empty literal text.
#[derive(Debug)]
pub struct Clusters {
    pub(crate) set: Vec<LiteralSet>,
    warnings: Vec<ExtractionWarning>,
    budget: CfgBudget,
}

impl Clusters {
    /// Source-spanned warnings for skipped documentation and sampled cfg coverage.
    pub fn warnings(&self) -> &[ExtractionWarning] {
        &self.warnings
    }

    /// Read attributes by ownership, combining an item's outer docs with its body's
    /// leading inner docs while keeping crate docs and child items independent.
    pub fn parse_token_tree(&mut self, source: &str, stream: TokenStream) -> Result<()> {
        parser::parse(
            source,
            stream,
            &mut self.set,
            &mut self.warnings,
            self.budget,
        )
    }

    /// Append developer-comment literal sets grouped by source adjacency.
    fn parse_developer_comments(&mut self, source: &str) {
        let developer_comments = extract_developer_comments(source);
        self.set.extend(developer_comments);
    }

    /// Sort literal sets by ascending source coverage so earlier comments appear first.
    fn ensure_sorted(&mut self) {
        self.set.sort_by_key(|ls| ls.coverage);
    }

    /// Extract the requested documentation and developer comments with default cfg limits.
    pub fn load_from_str(source: &str, doc_comments: bool, dev_comments: bool) -> Result<Self> {
        Self::load_from_str_with_budget(source, doc_comments, dev_comments, CfgBudget::default())
    }

    /// Extract the requested comment kinds with explicit per-owner cfg limits.
    ///
    /// Documentation selections containing unexpanded macros are skipped with warnings.
    /// Developer comments use source adjacency and do not consume the cfg budget.
    ///
    /// ```
    /// use doc_chunks::{CfgBudget, CheckableChunk, Clusters, ExtractionWarningKind};
    ///
    /// let source = r#"/// Transport.
    /// #[cfg_attr(tls, doc = "Encrypted.")]
    /// struct Transport;"#;
    /// let budget = CfgBudget { variants: 1, samples: 2, ..Default::default() };
    /// let expected_kind = ExtractionWarningKind::CfgBudget;
    /// let expected_variants = 2;
    /// let extraction = Clusters::load_from_str_with_budget(source, true, false, budget)?;
    /// assert_eq!(extraction.warnings()[0].kind, expected_kind);
    /// let chunks = Vec::<CheckableChunk>::from(extraction);
    /// assert_eq!(chunks.len(), expected_variants);
    /// # Ok::<(), doc_chunks::Error>(())
    /// ```
    pub fn load_from_str_with_budget(
        source: &str,
        doc_comments: bool,
        dev_comments: bool,
        budget: CfgBudget,
    ) -> Result<Self> {
        let mut chunk = Self {
            set: Vec::with_capacity(64),
            warnings: Vec::new(),
            budget,
        };
        if doc_comments {
            let stream = source
                .parse::<proc_macro2::TokenStream>()
                .map_err(|error| Error::ParserFailure(syn::Error::new(error.span(), error)))?;
            chunk.parse_token_tree(source, stream)?;
        }
        if dev_comments {
            chunk.parse_developer_comments(source);
        }
        chunk.ensure_sorted();
        Ok(chunk)
    }
}
