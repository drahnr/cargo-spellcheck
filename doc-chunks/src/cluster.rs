//! Cluster `proc_macro2::Literal`s into `LiteralSets`

use proc_macro2::Delimiter;
use syn::ext::IdentExt;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{parenthesized, Ident, LitStr, Macro, Token};

use super::cfg::{project, Predicate, ProjectionSite};
use super::{LiteralSet, TokenTree, TrimmedLiteral};
use crate::developer::extract_developer_comments;
use crate::errors::*;
use crate::{CfgBudget, Span};

enum DocComment {
    LitStr(LitStr),
    Macro(Macro),
}

impl DocComment {
    fn into_literal(self, source: &str) -> Result<TrimmedLiteral> {
        match self {
            Self::LitStr(literal) => TrimmedLiteral::load_from(source, Span::from(literal.span())),
            Self::Macro(mac) => Ok(TrimmedLiteral::new_empty(
                source,
                Span::from(mac.span()),
                crate::CommentVariant::MacroDocEqMacro,
            )),
        }
    }
}

impl syn::parse::Parse for DocComment {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        if name.unraw() != "doc" {
            return Err(syn::Error::new_spanned(name, "expected a doc attribute"));
        }
        input.parse::<Token![=]>()?;
        if input.peek(LitStr) {
            input.parse().map(Self::LitStr)
        } else {
            input.parse().map(Self::Macro)
        }
    }
}

/// Documentation attributes and their directly attached visibility predicates.
/// Other payloads are opaque, even if they contain tokens resembling `doc = ...`.
enum DocAttribute {
    Doc(DocComment),
    Conditional(Predicate, Span, Vec<DocAttribute>),
    Cfg(Predicate, Span),
    Other,
}

/// Parse a predicate while retaining its complete source span, including groups.
fn parse_spanned_predicate(input: syn::parse::ParseStream<'_>) -> syn::Result<(Predicate, Span)> {
    let mut cursor = input.cursor();
    let predicate = input.parse::<Predicate>()?;
    let end = input.cursor();
    let mut span = cursor.span();
    while cursor != end {
        let (token, next) = cursor
            .token_tree()
            .expect("a parsed predicate contains tokens");
        span = span.join(token.span()).unwrap_or(span);
        cursor = next;
    }
    let mut span = Span::from(span);
    // proc_macro2's end is exclusive; source extraction uses inclusive columns.
    span.end.column = span.end.column.saturating_sub(1);
    Ok((predicate, span))
}

impl syn::parse::Parse for DocAttribute {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let name = input
            .fork()
            .parse::<Ident>()
            .ok()
            .map(|name| name.unraw().to_string());
        if name.as_deref() == Some("doc") && input.peek2(Token![=]) {
            return input.parse().map(Self::Doc);
        }
        if name.as_deref() == Some("cfg") && input.peek2(syn::token::Paren) {
            input.parse::<Ident>()?;
            let content;
            parenthesized!(content in input);
            let (predicate, span) = parse_spanned_predicate(&content)?;
            if !content.is_empty() {
                return Err(content.error("expected a single cfg predicate"));
            }
            return Ok(Self::Cfg(predicate, span));
        }
        if name.as_deref() == Some("cfg_attr") && input.peek2(syn::token::Paren) {
            input.parse::<Ident>()?;
            let content;
            parenthesized!(content in input);
            let (predicate, span) = parse_spanned_predicate(&content)?;
            content.parse::<Token![,]>()?;
            let attributes = Punctuated::<DocAttribute, Token![,]>::parse_terminated(&content)?;
            return Ok(Self::Conditional(
                predicate,
                span,
                attributes.into_iter().collect(),
            ));
        }
        // A token group is consumed whole, so commas inside an unrelated
        // attribute's arguments cannot be mistaken for cfg_attr separators.
        while !input.is_empty() && !input.peek(Token![,]) {
            input.parse::<TokenTree>()?;
        }
        Ok(Self::Other)
    }
}

/// A symbolic predicate and the source location responsible for its coverage cost.
struct SpannedPredicate {
    predicate: Predicate,
    span: Span,
}

#[derive(Default)]
struct DocRun {
    docs: Vec<(SpannedPredicate, TrimmedLiteral)>,
    constraints: Vec<SpannedPredicate>,
    has_cfg: bool,
    non_reflowable: bool,
    intervening_attribute: bool,
    incomplete: bool,
}

impl DocRun {
    fn append(
        &mut self,
        source: &str,
        attribute: DocAttribute,
        guard: Predicate,
        guard_span: Option<Span>,
    ) -> Result<()> {
        match attribute {
            DocAttribute::Doc(comment) => {
                let literal = comment.into_literal(source)?;
                if let Some((_, previous)) = self.docs.last() {
                    // Structural adjacency can cross syntax that reflow must
                    // never replace (or put several attributes on one line).
                    self.non_reflowable |= self.intervening_attribute
                        || literal.span().start.line != previous.span().end.line + 1
                        || literal.variant() != previous.variant();
                }
                if let crate::CommentVariant::MacroDocEqStr(prefix, _) = literal.variant() {
                    // Reflow repeats this prefix. It must not contain another
                    // item or be a fragment of a multiline attribute wrapper.
                    let prefix: String = prefix.chars().filter(|c| !c.is_whitespace()).collect();
                    self.non_reflowable |= !matches!(
                        prefix.as_str(),
                        "#[doc=" | "#![doc=" | "#[r#doc=" | "#![r#doc="
                    );
                }
                self.intervening_attribute = false;
                let span = guard_span.unwrap_or_else(|| literal.span());
                self.docs.push((
                    SpannedPredicate {
                        predicate: guard,
                        span,
                    },
                    literal,
                ));
            }
            DocAttribute::Conditional(predicate, span, attributes) => {
                let span = guard_span.map_or(span, |parent| Span {
                    start: parent.start,
                    end: span.end,
                });
                self.has_cfg = true;
                let previous_len = self.docs.len();
                let guard = Predicate::All(vec![guard, predicate]);
                for attribute in attributes {
                    self.append(source, attribute, guard.clone(), Some(span))?;
                }
                self.non_reflowable |= self.docs.len() != previous_len;
                self.intervening_attribute = true;
            }
            DocAttribute::Cfg(predicate, span) => {
                let span = guard_span.map_or(span, |parent| Span {
                    start: parent.start,
                    end: span.end,
                });
                // TODO: propagate enclosing item/module guards and model the
                // order-dependent semantics of crate-level inner cfg attributes.
                self.has_cfg = true;
                // A cfg introduced by cfg_attr only constrains the item when
                // that attribute's parent guard applies: guard => predicate.
                let predicate = Predicate::Any(vec![Predicate::Not(Box::new(guard)), predicate]);
                self.constraints.push(SpannedPredicate { predicate, span });
                self.intervening_attribute = true;
            }
            DocAttribute::Other => self.intervening_attribute = true,
        }
        Ok(())
    }

    fn finish(
        self,
        sets: &mut Vec<LiteralSet>,
        warnings: &mut Vec<CfgCoverageWarning>,
        budget: CfgBudget,
    ) {
        // An incomplete attribute parse could omit a fence and expose example code as prose.
        if self.docs.is_empty() || self.incomplete {
            return;
        }
        // Unconditional documentation needs no symbolic projection.
        if !self.has_cfg {
            let literals = self.docs.into_iter().map(|(_, literal)| literal).collect();
            sets.push(LiteralSet::from_docs(literals, self.non_reflowable));
            return;
        }
        // Project complete alternatives using this run's explicit cfg budget.
        let report = project(
            self.docs.iter().map(|(guard, _)| &guard.predicate),
            self.constraints
                .iter()
                .map(|constraint| &constraint.predicate),
            budget,
        );
        // Locate incomplete-coverage warnings at the responsible source condition.
        if let Some(warning) = report.warning {
            let span = match warning.site {
                ProjectionSite::Guard(index) => self.docs[index].0.span,
                ProjectionSite::Constraint(index) => self.constraints[index].span,
            };
            let message = format!(
                "incomplete cfg documentation coverage: {}; checked {} complete documentation variants from a bounded sample of configurations",
                warning.error, report.projections.len(),
            );
            let message = if report.projections.is_empty() {
                format!("{message}; no satisfying nonempty documentation variant was found in the sample, which does not prove the documentation unreachable")
            } else {
                message
            };
            warnings.push(CfgCoverageWarning { span, message });
        }
        // Assemble whole selected fragments with their source mappings and cfg witness.
        for projection in report.projections {
            let literals = projection
                .selected
                .into_iter()
                .map(|index| self.docs[index].1.clone())
                .collect();
            let mut set = LiteralSet::from_docs(literals, self.non_reflowable);
            log::debug!(
                "Documentation at line {}: example cfg (symbolic): {}",
                set.coverage.0,
                projection.example
            );
            set.set_cfg_example(projection.example);
            sets.push(set);
        }
    }
}

/// A source-spanned warning that only a subset of cfg documentation was checked.
#[derive(Debug, Clone)]
pub struct CfgCoverageWarning {
    /// Source range covering the cfg predicates that exceeded the symbolic budget.
    pub span: Span,
    /// Explanation of the budget limit and resulting incomplete coverage.
    pub message: String,
}

/// Cluster comments together, such they appear as continuous text blocks.
#[derive(Debug)]
pub struct Clusters {
    pub(crate) set: Vec<LiteralSet>,
    warnings: Vec<CfgCoverageWarning>,
    budget: CfgBudget,
}

impl Clusters {
    /// Source-spanned warnings for documentation checked with sampled cfg coverage.
    pub fn warnings(&self) -> &[CfgCoverageWarning] {
        &self.warnings
    }

    /// Read actual attributes, keeping each item's documentation together even
    /// across inactive or unrelated attributes. Ordinary tokens and scope or
    /// inner/outer attribute boundaries terminate the current documentation run.
    pub fn parse_token_tree(
        &mut self,
        source: &str,
        stream: proc_macro2::TokenStream,
    ) -> Result<()> {
        let mut iter = stream.into_iter().peekable();
        let mut run = DocRun::default();
        let mut inner_style = None;
        while let Some(tree) = iter.next() {
            if matches!(&tree, TokenTree::Punct(punct) if punct.as_char() == '#') {
                let inner = iter
                    .next_if(
                        |tree| matches!(tree, TokenTree::Punct(punct) if punct.as_char() == '!'),
                    )
                    .is_some();
                if let Some(TokenTree::Group(group)) = iter.next_if(
                    |tree| matches!(tree, TokenTree::Group(group) if group.delimiter() == Delimiter::Bracket),
                ) {
                    if inner_style.is_some() && inner_style != Some(inner) {
                        std::mem::take(&mut run).finish(&mut self.set, &mut self.warnings, self.budget);
                    }
                    inner_style = Some(inner);
                    match syn::parse2::<DocAttribute>(group.stream()) {
                        Ok(attribute) => run.append(source, attribute, Predicate::always(), None)?,
                        Err(error) => {
                            // Do not descend into an invalid/unsupported attribute
                            // and accidentally interpret its payload as docs.
                            log::warn!("Skipping documentation block with an unsupported attribute: {}", error);
                            // An omitted fragment could be a code fence. Do not
                            // spellcheck an incomplete Markdown projection.
                            run.incomplete = true;
                        }
                    }
                    continue;
                }
            }
            std::mem::take(&mut run).finish(&mut self.set, &mut self.warnings, self.budget);
            inner_style = None;
            if let TokenTree::Group(group) = tree {
                self.parse_token_tree(source, group.stream())?;
            }
        }
        run.finish(&mut self.set, &mut self.warnings, self.budget);
        Ok(())
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
        self.set.sort_by_key(|ls| ls.coverage);
    }

    /// Load clusters from a `&str`. Optionally loads developer comments as
    /// well.
    pub fn load_from_str(source: &str, doc_comments: bool, dev_comments: bool) -> Result<Self> {
        Self::load_from_str_with_budget(source, doc_comments, dev_comments, CfgBudget::default())
    }

    /// Load documentation with explicit per-run cfg reasoning and sampling limits.
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
    fn load_flags_keep_doc_and_developer_comments_independent() {
        struct Case {
            docs: bool,
            devs: bool,
            expected_count: usize,
        }
        let source = r#"/// Documentation
struct Example;
// Developer comment
"#;
        let cases = [
            Case {
                docs: false,
                devs: false,
                expected_count: 0,
            },
            Case {
                docs: true,
                devs: false,
                expected_count: 1,
            },
            Case {
                docs: false,
                devs: true,
                expected_count: 1,
            },
            Case {
                docs: true,
                devs: true,
                expected_count: 2,
            },
        ];
        for Case {
            docs,
            devs,
            expected_count,
        } in cases
        {
            let clusters = Clusters::load_from_str(source, docs, devs).unwrap();
            assert_eq!(clusters.set.len(), expected_count);
        }
    }

    #[test]
    fn conditional_run_keeps_repeated_predicates_correlated() {
        let source = r#"
#[cfg_attr(feature = "a", doc = "First")]
#[cfg_attr(feature = "a", doc = "Second")]
#[cfg_attr(not(feature = "a"), doc = "Alternative")]
struct X;
"#;
        let clusters = Clusters::load_from_str(source, true, false).unwrap();
        let mut lengths: Vec<_> = clusters.set.iter().map(LiteralSet::len).collect();
        lengths.sort_unstable();
        assert_eq!(lengths, vec![1, 2]);
    }

    #[test]
    fn conditional_run_samples_on_selection_overflow_and_preserves_exact_coverage_below_it() {
        let budget = CfgBudget {
            variants: 4,
            samples: 4,
            ..CfgBudget::default()
        };
        struct Case {
            guard_count: usize,
            expected_count: Option<usize>,
            expected_warning: bool,
        }
        let cases = [
            Case {
                guard_count: 2,
                expected_count: Some(3),
                expected_warning: false,
            },
            Case {
                guard_count: 3,
                expected_count: None,
                expected_warning: true,
            },
        ];
        let expected_max_projections = budget.samples;
        for Case {
            guard_count,
            expected_count,
            expected_warning,
        } in cases
        {
            let mut source = String::new();
            for index in 0..guard_count {
                source.push_str(&format!(
                    r#"#[cfg_attr(flag{index}, doc = "Text")]
"#
                ));
            }
            source.push_str("struct SelectionBudget;");
            let clusters =
                Clusters::load_from_str_with_budget(&source, true, false, budget).unwrap();
            if let Some(expected_count) = expected_count {
                assert_eq!(clusters.set.len(), expected_count);
            }
            assert!(!clusters.set.is_empty());
            assert!(clusters.set.len() <= expected_max_projections);
            assert_eq!(!clusters.warnings().is_empty(), expected_warning);
        }
    }

    #[test]
    fn sampled_coverage_warning_identifies_the_overflowing_guard() {
        let source = r#"/// Handbook.
#[cfg_attr(chapter0, doc = "Introduction.")]
#[cfg_attr(chapter1, doc = "Installation.")]
#[cfg_attr(chapter2, doc = "Configuration.")]
struct Handbook;"#;
        let budget = CfgBudget {
            variants: 4,
            samples: 4,
            ..CfgBudget::default()
        };
        let expected_predicate = "chapter2";
        let expected_warning_count = 1;
        let clusters = Clusters::load_from_str_with_budget(source, true, false, budget).unwrap();
        assert!(!clusters.set.is_empty());
        assert_eq!(clusters.warnings().len(), expected_warning_count);
        let warning = &clusters.warnings()[0];
        let actual_predicate =
            crate::util::load_span_from(source.as_bytes(), warning.span).unwrap();
        assert_eq!(actual_predicate, expected_predicate);
        assert!(warning
            .message
            .contains("incomplete cfg documentation coverage"));
        assert!(warning.message.contains("bounded sample"));
    }

    #[test]
    fn nested_budget_warning_spans_include_the_expensive_parent_condition() {
        let budget = CfgBudget {
            variables: 2,
            samples: 4,
            ..CfgBudget::default()
        };
        // Cases pair Rust source with the expected final token in the warning span.
        let cases = [
            (
                r#"/// Transport.
#[cfg_attr(
    any(offline, tcp, tls),
    cfg_attr(inner_switch, doc = "Advanced setup."),
)]
struct Transport;"#,
                "inner_switch",
            ),
            (
                r#"/// Monitoring.
#[cfg_attr(
    any(stopped, starting, running),
    cfg(true),
)]
struct Monitoring;"#,
                "true",
            ),
        ];
        let expected_span_prefix = "any(";
        let expected_warning_count = 1;
        let expected_start_line = 3;
        for (source, expected_span_suffix) in cases {
            let clusters =
                Clusters::load_from_str_with_budget(source, true, false, budget).unwrap();
            assert!(!clusters.set.is_empty());
            assert_eq!(clusters.warnings().len(), expected_warning_count);
            let warning = &clusters.warnings()[0];
            let actual_span_content =
                crate::util::load_span_from(source.as_bytes(), warning.span).unwrap();
            assert_eq!(warning.span.start.line, expected_start_line);
            assert!(actual_span_content.starts_with(expected_span_prefix));
            assert!(actual_span_content.ends_with(expected_span_suffix));
            assert!(actual_span_content.contains("cfg"));
        }
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
}
