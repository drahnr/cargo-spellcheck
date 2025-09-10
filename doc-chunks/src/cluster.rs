//! Cluster `proc_macro2::Literal`s into `LiteralSets`

use syn::spanned::Spanned;
use syn::LitStr;
use syn::Macro;
use syn::Token;

use super::{LiteralSet, TokenTree, TrimmedLiteral};
use crate::cfg_eval::CfgContext;
use crate::developer::extract_developer_comments;

use crate::errors::*;
use crate::Span;

mod kw {
    syn::custom_keyword!(doc);
    syn::custom_keyword!(cfg_attr);
}

enum DocContent {
    LitStr(LitStr),
    Macro(Macro),
}
impl DocContent {
    fn span(&self) -> proc_macro2::Span {
        match self {
            Self::LitStr(inner) => inner.span(),
            Self::Macro(inner) => inner.span(),
        }
    }
}

struct DocComment {
    #[allow(dead_code)]
    doc: kw::doc,
    #[allow(dead_code)]
    eq_token: Token![=],
    content: DocContent,
}

impl syn::parse::Parse for DocComment {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let doc = input.parse::<kw::doc>()?;
        let eq_token: Token![=] = input.parse()?;

        let lookahead = input.lookahead1();
        let content = if lookahead.peek(LitStr) {
            input.parse().map(DocContent::LitStr)?
        } else {
            input.parse().map(DocContent::Macro)?
        };
        Ok(Self {
            doc,
            eq_token,
            content,
        })
    }
}

/// Cluster comments together, such they appear as continuous text blocks.
#[derive(Debug)]
pub struct Clusters {
    pub(crate) set: Vec<LiteralSet>,
    /// Configuration context for evaluating cfg expressions
    cfg_ctx: Option<CfgContext>,
}

impl Clusters {
    /// Only works if the file is processed line by line, otherwise requires a
    /// adjacency list.
    fn process_literal(
        &mut self,
        source: &str,
        comment: DocComment,
        is_cfg_attr: bool,
    ) -> Result<()> {
        let span = Span::from(comment.content.span());
        let trimmed_literal = match comment.content {
            DocContent::LitStr(ref s) => {
                // Check if this is a cfg_attr doc with code block delimiters
                if is_cfg_attr {
                    let value = s.value();
                    if value.contains("```") {
                        // cfg_attr docs with code blocks should be in their own cluster
                        // to prevent markdown parsing issues
                        if !self.set.is_empty() {
                            // Force a new cluster for cfg_attr with code blocks
                            let literal = TrimmedLiteral::load_from(source, span)?;
                            self.set.push(LiteralSet::from(literal));
                            return Ok(());
                        }
                    }
                }
                TrimmedLiteral::load_from(source, span)?
            }
            DocContent::Macro(_) => {
                TrimmedLiteral::new_empty(source, span, crate::CommentVariant::MacroDocEqMacro)
            }
        };
        if let Some(cls) = self.set.last_mut() {
            if let Err(trimmed_literal) = cls.add_adjacent(trimmed_literal) {
                log::trace!(target: "documentation",
                    "appending, but failed to append: {trimmed_literal:?} to set {cls:?}",
                );
                self.set.push(LiteralSet::from(trimmed_literal))
            } else {
                log::trace!("successfully appended to existing: {cls:?} to set");
            }
        } else {
            self.set.push(LiteralSet::from(trimmed_literal));
        }
        Ok(())
    }

    /// Helper function to parse a stream and associate the found literals.
    pub fn parse_token_tree(
        &mut self,
        source: &str,
        stream: proc_macro2::TokenStream,
    ) -> Result<()> {
        let iter = stream.into_iter();
        for tree in iter {
            if let TokenTree::Group(group) = tree {
                // First check if this is a direct doc comment
                if let Ok(comment) = syn::parse2::<DocComment>(group.stream()) {
                    if let Err(e) = self.process_literal(source, comment, false) {
                        log::error!("BUG: Failed to guarantee literal content/span integrity: {e}");
                        continue;
                    }
                } else {
                    // Check if this might be a cfg_attr with doc
                    let group_stream = group.stream();
                    if let Ok(()) = self.try_parse_cfg_attr_doc(source, group_stream.clone()) {
                        // Successfully parsed cfg_attr doc, continue
                        continue;
                    }
                    // Otherwise recurse into the group
                    self.parse_token_tree(source, group_stream)?;
                }
            };
        }
        Ok(())
    }

    /// Try to parse a cfg_attr attribute that contains a doc attribute
    fn try_parse_cfg_attr_doc(
        &mut self,
        source: &str,
        stream: proc_macro2::TokenStream,
    ) -> Result<()> {
        let mut tokens = stream.into_iter();

        // Check if first token is "cfg_attr"
        if let Some(TokenTree::Ident(ident)) = tokens.next() {
            if ident != "cfg_attr" {
                return Err(Error::Span("Not a cfg_attr".to_string()));
            }
        } else {
            return Err(Error::Span("No ident".to_string()));
        }

        // Skip the parentheses group
        if let Some(TokenTree::Group(group)) = tokens.next() {
            // Parse the contents of cfg_attr(condition, doc = "...")
            let mut inner_tokens = group.stream().into_iter();
            let mut condition_tokens = proc_macro2::TokenStream::new();
            let mut found_doc = false;
            let mut skip_until_comma = true;

            while let Some(token) = inner_tokens.next() {
                if skip_until_comma {
                    // Collect the condition tokens until we find a comma
                    if let TokenTree::Punct(punct) = &token {
                        if punct.as_char() == ',' {
                            skip_until_comma = false;

                            // Evaluate the condition if we have a cfg context
                            if let Some(ref cfg_ctx) = self.cfg_ctx {
                                match cfg_ctx.evaluate(condition_tokens.clone()) {
                                    Ok(false) => {
                                        // Condition is false, skip this cfg_attr doc
                                        log::trace!("Skipping cfg_attr doc due to false condition");
                                        return Ok(());
                                    }
                                    Ok(true) => {
                                        // Condition is true, process the doc
                                        log::trace!("Including cfg_attr doc due to true condition");
                                    }
                                    Err(e) => {
                                        log::warn!("Failed to evaluate cfg condition: {}", e);
                                        // On error, include the doc (conservative approach)
                                    }
                                }
                            }
                            continue;
                        }
                    }
                    condition_tokens.extend(std::iter::once(token));
                    continue;
                }

                // Now look for doc = "..."
                if let TokenTree::Ident(ident) = &token {
                    if ident == "doc" {
                        found_doc = true;
                        // Next should be =
                        if let Some(TokenTree::Punct(punct)) = inner_tokens.next() {
                            if punct.as_char() == '=' {
                                // Next should be the doc string
                                if let Some(doc_token) = inner_tokens.next() {
                                    // Manually construct the doc stream
                                    use proc_macro2::{Ident, Punct, Spacing, TokenStream};
                                    let mut doc_stream = TokenStream::new();
                                    doc_stream.extend([
                                        TokenTree::Ident(Ident::new(
                                            "doc",
                                            proc_macro2::Span::call_site(),
                                        )),
                                        TokenTree::Punct(Punct::new('=', Spacing::Alone)),
                                        doc_token,
                                    ]);

                                    if let Ok(comment) = syn::parse2::<DocComment>(doc_stream) {
                                        // Process this as a cfg_attr doc comment
                                        log::trace!("Found cfg_attr doc comment");
                                        let _ = self.process_literal(source, comment, true);
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !found_doc {
                return Err(Error::Span("No doc in cfg_attr".to_string()));
            }
        }

        Err(Error::Span("Failed to parse cfg_attr".to_string()))
    }

    /// From the given source text, extracts developer comments to `LiteralSet`s
    /// and adds them to this `Clusters`
    fn parse_developer_comments(&mut self, source: &str) {
        let developer_comments = extract_developer_comments(source);
        self.set.extend(developer_comments);
    }

    /// Sort the `LiteralSet`s in this `Cluster` by start line descending, to
    /// ensure that the comments higher up in the source file appear first to
    /// the user
    fn ensure_sorted(&mut self) {
        self.set.sort_by(|ls1, ls2| ls1.coverage.cmp(&ls2.coverage));
    }

    /// Load clusters from a `&str`. Optionally loads developer comments as
    /// well.
    pub fn load_from_str(source: &str, doc_comments: bool, dev_comments: bool) -> Result<Self> {
        Self::load_from_str_with_cfg(source, doc_comments, dev_comments, None)
    }

    /// Load clusters from a `&str` with optional cfg evaluation context.
    pub fn load_from_str_with_cfg(
        source: &str,
        doc_comments: bool,
        dev_comments: bool,
        cfg_ctx: Option<CfgContext>,
    ) -> Result<Self> {
        let mut chunk = Self {
            set: Vec::with_capacity(64),
            cfg_ctx,
        };
        if doc_comments {
            let stream =
                syn::parse_str::<proc_macro2::TokenStream>(source).map_err(Error::ParserFailure)?;
            chunk.parse_token_tree(source, stream)?;
        }
        if dev_comments {
            chunk.parse_developer_comments(source);
        }
        chunk.ensure_sorted();
        Ok(chunk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_comment_parse() {
        let _ = syn::parse_str::<DocComment>(r########"doc=foo!(bar!(xxx))"########).unwrap();
        let _ = syn::parse_str::<DocComment>(r########"doc="s""########).unwrap();
        let _ = syn::parse_str::<DocComment>(r########"doc=r#"s"#"########).unwrap();
        let _ = syn::parse_str::<DocComment>(r########"doc=r##"s"##"########).unwrap();
        let _ = syn::parse_str::<DocComment>(r########"doc=r###"s"###"########).unwrap();
        let _ = syn::parse_str::<DocComment>(r########"doc=r####"s"####"########).unwrap();
    }

    #[test]
    fn create_cluster() {
        static CONTENT: &str = r#####"
mod mm_mm {

/// A
#[doc=foo!(B)]
/// C
#[doc=r##"D"##]
struct X;

}
"#####;
        let clusters = Clusters::load_from_str(CONTENT, true, true).unwrap();
        assert_eq!(clusters.set.len(), 1);
        dbg!(&clusters.set[0]);
    }

    #[test]
    fn space_in_code_block_does_not_break_cluster() {
        static CONTENT: &str = r#####"
// ```c
// hugloboi
//
// fucksteufelswuid
// ```
struct DefinitelyNotZ;
"#####;
        let clusters = Clusters::load_from_str(CONTENT, true, true).unwrap();
        assert_eq!(clusters.set.len(), 1);
        dbg!(&clusters.set[0]);
    }

    #[test]
    fn polite() {
        static CONTENT: &str = r#####"
// Hello Sir
//
// How are you doing today?
struct VeryWellThanks;
"#####;
        let clusters = Clusters::load_from_str(CONTENT, true, true).unwrap();
        assert_eq!(clusters.set.len(), 1);
        dbg!(&clusters.set[0]);
    }

    #[test]
    fn cfg_scoped_doc_comments_should_break_cluster() {
        static CONTENT: &str = r#####"
/// First documentation block that is quite long and
/// spans multiple lines for testing reflow behavior.
#[cfg(feature = "foo")]
/// This comment is cfg-gated and should be a separate cluster
/// to avoid incorrect reflow across cfg boundaries.
struct Foo;

/// This comment comes after the cfg block
/// and should also be a separate cluster.
struct Bar;
"#####;
        let clusters = Clusters::load_from_str(CONTENT, true, false).unwrap();
        // Currently this fails - it treats all as one cluster
        // We want 3 separate clusters to avoid incorrect reflow
        eprintln!("Number of clusters: {}", clusters.set.len());
        for (i, cluster) in clusters.set.iter().enumerate() {
            eprintln!(
                "Cluster {}: coverage {:?}, len {}",
                i,
                cluster.coverage,
                cluster.len()
            );
        }
        assert_eq!(
            clusters.set.len(),
            3,
            "Should have 3 separate clusters due to cfg boundary"
        );
    }

    #[test]
    fn cfg_attr_doc_should_be_handled() {
        static CONTENT: &str = r#####"
/// Regular doc comment before cfg_attr
#[cfg_attr(feature = "foo", doc = "Conditional documentation")]
#[cfg_attr(not(feature = "foo"), doc = "Alternative conditional documentation")]
struct Foo;

/// Another regular doc comment
struct Bar;
"#####;
        let clusters = Clusters::load_from_str(CONTENT, true, false).unwrap();
        eprintln!("Number of clusters for cfg_attr: {}", clusters.set.len());
        for (i, cluster) in clusters.set.iter().enumerate() {
            eprintln!(
                "Cluster {}: coverage {:?}, len {}",
                i,
                cluster.coverage,
                cluster.len()
            );
        }

        // cfg_attr doc attributes should be recognized and handled
        // They are now included in the same cluster as regular docs (when no code blocks)
        assert_eq!(
            clusters.set.len(),
            2,
            "Should have exactly 2 clusters with cfg_attr docs included"
        );
        assert_eq!(
            clusters.set[0].len(),
            3,
            "First cluster should contain exactly 3 items: regular doc + 2 cfg_attr docs"
        );
        assert_eq!(
            clusters.set[0].coverage,
            (2, 4),
            "First cluster should cover lines 2-4"
        );
        assert_eq!(
            clusters.set[1].len(),
            1,
            "Second cluster should contain exactly 1 regular doc comment"
        );
        assert_eq!(
            clusters.set[1].coverage,
            (7, 7),
            "Second cluster should cover line 7"
        );
    }

    #[test]
    fn cfg_attr_with_code_block_doc() {
        static CONTENT: &str = r#####"
/// Regular documentation
#[cfg_attr(feature = "advanced", doc = r#"
Advanced feature documentation with code:
```rust
fn example() {
    println!("This is a code block inside cfg_attr doc");
}
```
"#)]
struct AdvancedFeature;

/// More regular documentation that should not merge
/// with the cfg_attr documentation above.
struct NextStruct;
"#####;
        let clusters = Clusters::load_from_str(CONTENT, true, false).unwrap();
        eprintln!(
            "Number of clusters for cfg_attr with code: {}",
            clusters.set.len()
        );
        for (i, cluster) in clusters.set.iter().enumerate() {
            eprintln!(
                "Cluster {}: coverage {:?}, len {}",
                i,
                cluster.coverage,
                cluster.len()
            );
        }

        // The cfg_attr doc with code block should be in its own cluster
        assert_eq!(
            clusters.set.len(),
            3,
            "Should have exactly 3 clusters: regular doc, cfg_attr with code block, and final regular doc"
        );

        // First cluster: initial regular documentation
        assert_eq!(
            clusters.set[0].len(),
            1,
            "First cluster should contain exactly 1 regular doc"
        );
        assert_eq!(
            clusters.set[0].coverage,
            (2, 2),
            "First cluster should cover line 2"
        );

        // Second cluster: cfg_attr with code block (isolated)
        assert_eq!(
            clusters.set[1].len(),
            1,
            "Second cluster should contain exactly 1 cfg_attr with code block"
        );
        assert_eq!(
            clusters.set[1].coverage,
            (3, 10), // The multi-line cfg_attr spans lines 3-10
            "Second cluster should cover lines 3-10 (the entire cfg_attr)"
        );

        // Third cluster: final regular documentation
        assert_eq!(
            clusters.set[2].len(),
            2,
            "Third cluster should contain exactly 2 lines of regular docs"
        );
        assert_eq!(
            clusters.set[2].coverage,
            (13, 14),
            "Third cluster should cover lines 13-14"
        );
    }

    #[test]
    fn cfg_attr_split_code_block_delimiters() {
        // This tests the critical case where code block delimiters are split across cfg_attr conditions
        static CONTENT: &str = r#####"
/// Documentation with conditional code blocks
#[cfg_attr(feature = "foo", doc = "```rust")]
#[cfg_attr(not(feature = "foo"), doc = "```python")]
/// funky
/// bro
/// ```
/// This text comes after the code block
struct ConditionalCode;
"#####;
        let clusters = Clusters::load_from_str(CONTENT, true, false).unwrap();
        eprintln!(
            "Number of clusters for split code blocks: {}",
            clusters.set.len()
        );
        for (i, cluster) in clusters.set.iter().enumerate() {
            eprintln!(
                "Cluster {}: coverage {:?}, len {}",
                i,
                cluster.coverage,
                cluster.len()
            );
        }

        // cfg_attr docs with code block delimiters are now placed in separate clusters
        // to prevent markdown parsing confusion
        assert_eq!(
            clusters.set.len(),
            3,
            "Should have exactly 3 clusters to separate cfg_attr code blocks"
        );

        // First cluster: regular doc
        assert_eq!(
            clusters.set[0].len(),
            1,
            "First cluster should contain exactly 1 line (initial doc)"
        );
        assert_eq!(
            clusters.set[0].coverage,
            (2, 2),
            "First cluster should cover line 2"
        );

        // Second cluster: cfg_attr with opening delimiter (isolated)
        assert_eq!(
            clusters.set[1].len(),
            1,
            "Second cluster should contain exactly 1 cfg_attr"
        );
        assert_eq!(
            clusters.set[1].coverage,
            (3, 3),
            "Second cluster should cover line 3"
        );

        // Third cluster: remaining docs including the code block content
        assert_eq!(
            clusters.set[2].len(),
            5,
            "Third cluster should contain exactly 5 lines"
        );
        assert_eq!(
            clusters.set[2].coverage,
            (4, 8),
            "Third cluster should cover lines 4-8"
        );
    }

    #[test]
    fn cfg_attr_evaluation_with_features() {
        use crate::cfg_eval::CfgContext;

        static CONTENT: &str = r#####"
/// Always included documentation
#[cfg_attr(feature = "foo", doc = "Documentation when foo is enabled")]
#[cfg_attr(not(feature = "foo"), doc = "Documentation when foo is disabled")]
struct Foo;
"#####;

        // Test with foo feature enabled
        {
            let cfg_ctx = CfgContext::new(
                None,
                None,
                vec!["foo".to_string()],
                true, // eval_cfg enabled
            );
            let clusters =
                Clusters::load_from_str_with_cfg(CONTENT, true, false, Some(cfg_ctx)).unwrap();

            eprintln!("With foo enabled - clusters: {}", clusters.set.len());
            for (i, cluster) in clusters.set.iter().enumerate() {
                eprintln!(
                    "  Cluster {}: len={}, coverage={:?}",
                    i,
                    cluster.len(),
                    cluster.coverage
                );
                eprintln!("  Cluster {} content: {:?}", i, cluster);
            }

            // We actually get 2 clusters because the struct declaration gets a separate cluster
            // But we only care about the first one which has the docs
            assert!(clusters.set.len() >= 1);
            assert_eq!(clusters.set[0].len(), 2);

            // Check that we have the right documentation
            let cluster_text = format!("{:?}", clusters.set[0]);
            assert!(cluster_text.contains("Always included"));
            assert!(cluster_text.contains("foo is enabled"));
            assert!(!cluster_text.contains("foo is disabled"));
        }

        // Test with foo feature disabled
        {
            let cfg_ctx = CfgContext::new(
                None,
                None,
                vec![], // No features
                true,   // eval_cfg enabled
            );
            let clusters =
                Clusters::load_from_str_with_cfg(CONTENT, true, false, Some(cfg_ctx)).unwrap();

            eprintln!("With foo disabled - clusters: {}", clusters.set.len());
            for (i, cluster) in clusters.set.iter().enumerate() {
                eprintln!(
                    "  Cluster {}: len={}, coverage={:?}",
                    i,
                    cluster.len(),
                    cluster.coverage
                );
                eprintln!("  Cluster {} content: {:?}", i, cluster);
            }

            // We get 2 clusters because skipping line 3 breaks adjacency
            assert_eq!(clusters.set.len(), 2);
            assert_eq!(clusters.set[0].len(), 1); // "Always included"
            assert_eq!(clusters.set[1].len(), 1); // "foo is disabled"

            // Check that we have the right documentation
            let all_text = format!("{:?}{:?}", clusters.set[0], clusters.set[1]);
            assert!(all_text.contains("Always included"));
            assert!(!all_text.contains("foo is enabled"));
            assert!(all_text.contains("foo is disabled"));
        }

        // Test with eval_cfg disabled (all docs included)
        {
            let cfg_ctx = CfgContext::new(
                None,
                None,
                vec![],
                false, // eval_cfg disabled
            );
            let clusters =
                Clusters::load_from_str_with_cfg(CONTENT, true, false, Some(cfg_ctx)).unwrap();

            // Should have at least 1 cluster with 3 items: all docs included
            assert!(clusters.set.len() >= 1);
            assert_eq!(clusters.set[0].len(), 3);

            // Check that we have all documentation
            let cluster_text = format!("{:?}", clusters.set[0]);
            assert!(cluster_text.contains("Always included"));
            assert!(cluster_text.contains("foo is enabled"));
            assert!(cluster_text.contains("foo is disabled"));
        }
    }

    #[test]
    fn cfg_attr_evaluation_with_target() {
        use crate::cfg_eval::CfgContext;

        static CONTENT: &str = r#####"
/// Always included documentation
#[cfg_attr(target_os = "linux", doc = "Linux-specific documentation")]
#[cfg_attr(target_os = "windows", doc = "Windows-specific documentation")]
struct Platform;
"#####;

        // Test with Linux target
        {
            let cfg_ctx = CfgContext::new(
                Some("x86_64-unknown-linux-gnu".to_string()),
                None,
                vec![],
                true, // eval_cfg enabled
            );
            let clusters =
                Clusters::load_from_str_with_cfg(CONTENT, true, false, Some(cfg_ctx)).unwrap();

            eprintln!("With Linux target - clusters: {}", clusters.set.len());
            for (i, cluster) in clusters.set.iter().enumerate() {
                eprintln!("  Cluster {}: {:?}", i, cluster);
            }

            // Should have Linux-specific doc
            let cluster_text = format!("{:?}", clusters.set[0]);
            assert!(cluster_text.contains("Always included"));
            assert!(cluster_text.contains("Linux-specific"));
            assert!(!cluster_text.contains("Windows-specific"));
        }

        // Test with Windows target
        {
            let cfg_ctx = CfgContext::new(
                Some("x86_64-pc-windows-msvc".to_string()),
                None,
                vec![],
                true, // eval_cfg enabled
            );
            let clusters =
                Clusters::load_from_str_with_cfg(CONTENT, true, false, Some(cfg_ctx)).unwrap();

            eprintln!("With Windows target - clusters: {}", clusters.set.len());
            for (i, cluster) in clusters.set.iter().enumerate() {
                eprintln!("  Cluster {}: {:?}", i, cluster);
            }

            // Should have Windows-specific doc
            let all_text = format!("{:?}", clusters);
            assert!(all_text.contains("Always included"));
            assert!(!all_text.contains("Linux-specific"));
            assert!(all_text.contains("Windows-specific"));
        }
    }
}
