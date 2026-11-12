//! Cluster `proc_macro2::Literal`s into `LiteralSets`

use std::collections::VecDeque;

use proc_macro2::{Delimiter, Group, TokenStream};
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
    Conditional {
        condition: SpannedPredicate,
        attributes: Vec<DocAttribute>,
    },
    Cfg(SpannedPredicate),
    Other,
}

/// Parse a predicate while retaining its complete source span, including groups.
fn parse_spanned_predicate(input: syn::parse::ParseStream<'_>) -> syn::Result<SpannedPredicate> {
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
    Ok(SpannedPredicate { predicate, span })
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
            let condition = parse_spanned_predicate(&content)?;
            content.parse::<Option<Token![,]>>()?;
            if !content.is_empty() {
                return Err(content.error("expected a single cfg predicate"));
            }
            return Ok(Self::Cfg(condition));
        }
        if name.as_deref() == Some("cfg_attr") && input.peek2(syn::token::Paren) {
            input.parse::<Ident>()?;
            let content;
            parenthesized!(content in input);
            let condition = parse_spanned_predicate(&content)?;
            content.parse::<Token![,]>()?;
            let attributes = Punctuated::<DocAttribute, Token![,]>::parse_terminated(&content)?;
            return Ok(Self::Conditional {
                condition,
                attributes: Vec::from_iter(attributes),
            });
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
#[derive(Clone)]
struct SpannedPredicate {
    predicate: Predicate,
    span: Span,
}

/// Nested attribute guards retain the source range of their combined condition.
#[derive(Clone, Default)]
struct GuardContext {
    condition: Option<SpannedPredicate>,
}

impl GuardContext {
    fn predicate(&self) -> Predicate {
        self.condition
            .as_ref()
            .map_or_else(Predicate::always, |condition| condition.predicate.clone())
    }

    fn covering_span(&self, span: Span) -> Span {
        self.condition.as_ref().map_or(span, |parent| Span {
            start: parent.span.start,
            end: span.end,
        })
    }

    fn nested(&self, condition: SpannedPredicate) -> Self {
        let span = self.covering_span(condition.span);
        Self {
            condition: Some(SpannedPredicate {
                predicate: Predicate::All(vec![self.predicate(), condition.predicate]),
                span,
            }),
        }
    }

    fn for_literal(&self, span: Span) -> SpannedPredicate {
        self.condition.clone().unwrap_or_else(|| SpannedPredicate {
            predicate: Predicate::always(),
            span,
        })
    }

    fn constrain(&self, condition: SpannedPredicate) -> SpannedPredicate {
        let span = self.covering_span(condition.span);
        // A generated cfg constrains its owner only when the parent guard applies.
        SpannedPredicate {
            predicate: Predicate::Any(vec![
                Predicate::Not(Box::new(self.predicate())),
                condition.predicate,
            ]),
            span,
        }
    }
}

/// Inner attributes document the enclosing owner; outer attributes document its next item.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AttributeStyle {
    Inner,
    Outer,
}

/// Inspect an attribute prefix without consuming unrelated token groups.
fn attribute_prefix(tokens: &VecDeque<TokenTree>) -> Option<(AttributeStyle, usize)> {
    if !matches!(tokens.front(), Some(TokenTree::Punct(punct)) if punct.as_char() == '#') {
        return None;
    }
    let (style, group_index) = match tokens.get(1) {
        Some(TokenTree::Punct(punct)) if punct.as_char() == '!' => (AttributeStyle::Inner, 2),
        _ => (AttributeStyle::Outer, 1),
    };
    match tokens.get(group_index) {
        Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Bracket => {
            Some((style, group_index))
        }
        _ => None,
    }
}

fn take_attribute(tokens: &mut VecDeque<TokenTree>) -> Option<(AttributeStyle, Group)> {
    let (style, group_index) = attribute_prefix(tokens)?;
    let Some(TokenTree::Group(group)) = tokens.get(group_index) else {
        unreachable!("an attribute prefix ends in a bracket group");
    };
    let group = group.clone();
    tokens.drain(..=group_index);
    Some((style, group))
}

/// Recognize an item's body without requiring its contents or the file to parse as Rust.
fn has_item_body(header: &TokenStream) -> bool {
    let mut candidate = header.clone();
    candidate.extend([TokenTree::Group(Group::new(
        Delimiter::Brace,
        TokenStream::new(),
    ))]);
    matches!(
        syn::parse2::<syn::Item>(candidate),
        Ok(syn::Item::Fn(_)
            | syn::Item::Mod(_)
            | syn::Item::Impl(_)
            | syn::Item::Trait(_)
            | syn::Item::Enum(_)
            | syn::Item::Struct(_)
            | syn::Item::Union(_)
            | syn::Item::ForeignMod(_)
            | syn::Item::Macro(_))
    )
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
    fn with_constraints(constraints: &[SpannedPredicate]) -> Self {
        Self {
            constraints: constraints.to_vec(),
            has_cfg: !constraints.is_empty(),
            ..Self::default()
        }
    }

    fn append_tokens(&mut self, source: &str, group: Group) -> Result<()> {
        match syn::parse2::<DocAttribute>(group.stream()) {
            Ok(attribute) => self.append(source, attribute, &GuardContext::default())?,
            Err(error) => {
                // An omitted fragment could be a fence; never descend into its payload.
                log::warn!(
                    "Skipping documentation block with an unsupported attribute: {}",
                    error
                );
                self.incomplete = true;
            }
        }
        Ok(())
    }

    fn append(
        &mut self,
        source: &str,
        attribute: DocAttribute,
        guard: &GuardContext,
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
                    let prefix = String::from_iter(prefix.chars().filter(|c| !c.is_whitespace()));
                    self.non_reflowable |= !matches!(
                        prefix.as_str(),
                        "#[doc=" | "#![doc=" | "#[r#doc=" | "#![r#doc="
                    );
                }
                self.intervening_attribute = false;
                self.docs.push((guard.for_literal(literal.span()), literal));
            }
            DocAttribute::Conditional {
                condition,
                attributes,
            } => {
                self.has_cfg = true;
                let previous_len = self.docs.len();
                let guard = guard.nested(condition);
                for attribute in attributes {
                    self.append(source, attribute, &guard)?;
                }
                self.non_reflowable |= self.docs.len() != previous_len;
                self.intervening_attribute = true;
            }
            DocAttribute::Cfg(condition) => {
                self.has_cfg = true;
                self.constraints.push(guard.constrain(condition));
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
            let literals = Vec::from_iter(self.docs.into_iter().map(|(_, literal)| literal));
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
            let literals = Vec::from_iter(
                projection
                    .selected
                    .into_iter()
                    .map(|index| self.docs[index].1.clone()),
            );
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

    /// Read attributes by ownership, combining an item's outer docs with its body's
    /// leading inner docs while keeping crate docs and child items independent.
    pub fn parse_token_tree(&mut self, source: &str, stream: TokenStream) -> Result<()> {
        self.parse_scope(source, stream, DocRun::default())
    }

    fn parse_scope(&mut self, source: &str, stream: TokenStream, mut owner: DocRun) -> Result<()> {
        let mut tokens = VecDeque::from_iter(stream);
        // Merge leading inner attributes before projecting the owner's complete Markdown.
        while matches!(attribute_prefix(&tokens), Some((AttributeStyle::Inner, _))) {
            let (_, group) = take_attribute(&mut tokens).unwrap();
            owner.append_tokens(source, group)?;
        }
        let enclosing = owner.constraints.clone();
        owner.finish(&mut self.set, &mut self.warnings, self.budget);

        // Start each sibling with only the enclosing owner's constraints.
        let mut run = DocRun::with_constraints(&enclosing);
        let mut header = TokenStream::new();
        let mut attribute_style = None;
        while !tokens.is_empty() {
            if let Some((style, group)) = take_attribute(&mut tokens) {
                if !header.is_empty() || attribute_style.is_some_and(|previous| previous != style) {
                    run.finish(&mut self.set, &mut self.warnings, self.budget);
                    run = DocRun::with_constraints(&enclosing);
                    header = TokenStream::new();
                }
                attribute_style = Some(style);
                run.append_tokens(source, group)?;
                continue;
            }
            let tree = tokens.pop_front().unwrap();
            match tree {
                TokenTree::Group(group)
                    if group.delimiter() == Delimiter::Brace && has_item_body(&header) =>
                {
                    // Body inner attributes constrain both the owner and its descendants.
                    self.parse_scope(source, group.stream(), run)?;
                    run = DocRun::with_constraints(&enclosing);
                    header = TokenStream::new();
                    attribute_style = None;
                }
                TokenTree::Group(group) => {
                    // Retain token-level traversal for expressions and malformed Rust samples.
                    let context = DocRun::with_constraints(&run.constraints);
                    self.parse_scope(source, group.stream(), context)?;
                    header.extend([TokenTree::Group(group)]);
                }
                TokenTree::Punct(punct) if punct.as_char() == ';' => {
                    run.finish(&mut self.set, &mut self.warnings, self.budget);
                    run = DocRun::with_constraints(&enclosing);
                    header = TokenStream::new();
                    attribute_style = None;
                }
                tree => header.extend([tree]),
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
