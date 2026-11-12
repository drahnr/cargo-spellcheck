//! Extract documentation fragments, group them by owner, and emit complete literal sets.
//!
//! `DocAssignment` represents parsed `doc = ...` syntax. `DocFragment` retains either
//! known text, an unexpanded macro's span, or a guarded unsupported payload. `OwnerDocumentation`
//! groups guarded fragments and emits `LiteralSet`s only for fully known selections;
//! macros are not expanded here. Developer comments are grouped by source adjacency.

use std::collections::{HashSet, VecDeque};

use proc_macro2::{Delimiter, Group, TokenStream};
use syn::ext::IdentExt;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{parenthesized, Expr, Ident, Lit, LitStr, Macro, Token};

use super::cfg::{project, Predicate, ProjectionSite};
use super::{LiteralSet, TokenTree, TrimmedLiteral};
use crate::developer::extract_developer_comments;
use crate::errors::*;
use crate::{CfgBudget, Span};

/// Parsed `doc = ...` syntax, before loading source-backed documentation fragments.
enum DocAssignment {
    LitStr(LitStr),
    Macro(Macro),
    NonDocLiteral,
    Unsupported(Span),
}

/// A source-backed fragment whose unknown content suppresses only selected cfg alternatives.
enum DocFragment {
    Literal(TrimmedLiteral),
    UnexpandedMacro(Span),
    Unsupported(ExtractionWarning),
}

impl DocFragment {
    fn span(&self) -> Span {
        match self {
            Self::Literal(literal) => literal.span(),
            Self::UnexpandedMacro(span) => *span,
            Self::Unsupported(warning) => warning.span,
        }
    }
}

impl DocAssignment {
    fn into_fragment(self, source: &str) -> Result<Option<DocFragment>> {
        match self {
            Self::LitStr(literal) => TrimmedLiteral::load_from(source, Span::from(literal.span()))
                .map(|literal| Some(DocFragment::Literal(literal))),
            Self::Macro(mac) => Ok(Some(DocFragment::UnexpandedMacro(inclusive_span(mac.span())))),
            Self::NonDocLiteral => Ok(None),
            Self::Unsupported(span) => Ok(Some(DocFragment::Unsupported(ExtractionWarning {
                kind: ExtractionWarningKind::UnsupportedAttribute,
                span,
                message: "Skipping documentation variants containing an unsupported doc expression; expected an unsuffixed string literal or an unexpanded macro".to_owned(),
            }))),
        }
    }
}

/// Convert an exclusive token span to the inclusive source coordinates used by chunks.
fn inclusive_span(span: proc_macro2::Span) -> Span {
    let mut span = Span::from(span);
    span.end.column = span.end.column.saturating_sub(1);
    span
}

impl syn::parse::Parse for DocAssignment {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        if name.unraw() != "doc" {
            return Err(syn::Error::new_spanned(name, "expected a doc attribute"));
        }
        input.parse::<Token![=]>()?;
        let expression = input.parse::<Expr>()?;
        Ok(match expression {
            Expr::Lit(literal) => match literal.lit {
                Lit::Str(literal) if literal.suffix().is_empty() => Self::LitStr(literal),
                Lit::Str(literal) => Self::Unsupported(inclusive_span(literal.span())),
                _ => Self::NonDocLiteral,
            },
            Expr::Macro(expression) => Self::Macro(expression.mac),
            expression => Self::Unsupported(inclusive_span(expression.span())),
        })
    }
}

/// Documentation assignments, conditional attribute lists, and owner cfg constraints.
/// Unrelated payloads are consumed without inspecting nested `doc = ...` tokens.
enum DocAttribute {
    Doc(DocAssignment),
    Conditional {
        condition: SpannedPredicate,
        attributes: Vec<DocAttribute>,
    },
    Cfg(SpannedPredicate),
    Unsupported(ExtractionWarning),
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
    Ok(SpannedPredicate {
        predicate,
        span: inclusive_span(span),
    })
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
        // Unsafe attribute wrappers are unrelated metadata, not nested doc/cfg attributes.
        if input.peek(Token![unsafe]) {
            input.parse::<Token![unsafe]>()?;
            let content;
            parenthesized!(content in input);
            content.parse::<TokenStream>()?;
            return Ok(Self::Other);
        }
        let path = input.call(syn::Path::parse_mod_style)?;
        let name = path.get_ident().map(|name| name.unraw().to_string());
        let mut span = path.span();
        let group = if input.peek(syn::token::Paren)
            || input.peek(syn::token::Bracket)
            || input.peek(syn::token::Brace)
        {
            let TokenTree::Group(group) = input.parse::<TokenTree>()? else {
                unreachable!("a delimiter starts a token group");
            };
            span = span.join(group.span()).unwrap_or(span);
            Some(group)
        } else if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            // Expression parsing owns closure and turbofish commas; never scan their tokens
            // as if they were cfg_attr separators or nested documentation attributes.
            let expression = input.parse::<Expr>()?;
            span = span.join(expression.span()).unwrap_or(span);
            None
        } else {
            None
        };
        if !matches!(name.as_deref(), Some("cfg" | "cfg_attr")) {
            return Ok(Self::Other);
        }
        let parsed = (|| {
            let group = group.ok_or_else(|| {
                syn::Error::new(span, "expected parenthesized configuration attribute")
            })?;
            if group.delimiter() != Delimiter::Parenthesis {
                return Err(syn::Error::new(
                    span,
                    "expected parentheses for configuration attribute",
                ));
            }
            (|content: syn::parse::ParseStream<'_>| {
                let condition = parse_spanned_predicate(content)?;
                if name.as_deref() == Some("cfg") {
                    content.parse::<Option<Token![,]>>()?;
                    if !content.is_empty() {
                        return Err(content.error("expected a single cfg predicate"));
                    }
                    Ok(Self::Cfg(condition))
                } else {
                    content.parse::<Token![,]>()?;
                    let payload = content.parse::<TokenStream>()?;
                    let attributes = match Punctuated::<DocAttribute, Token![,]>::parse_terminated
                        .parse2(payload.clone())
                    {
                        Ok(attributes) => Vec::from_iter(attributes),
                        // rustc need not validate discarded attribute expressions as eagerly as
                        // syn does. Preserve the parsed parent guard even if child parsing fails.
                        Err(error) => vec![Self::Unsupported(ExtractionWarning {
                            kind: ExtractionWarningKind::UnsupportedAttribute,
                            span: inclusive_span(payload.span()),
                            message: format!(
                                "Skipping documentation variants with unsupported attribute payloads: {error}"
                            ),
                        })],
                    };
                    Ok(Self::Conditional {
                        condition,
                        attributes,
                    })
                }
            })
            .parse2(group.stream())
        })();
        // Syntactically consumed payloads can be discarded by an enclosing cfg_attr.
        // Retain interpretation failures under that guard instead of poisoning the owner.
        Ok(parsed.unwrap_or_else(|error| {
            Self::Unsupported(ExtractionWarning {
                kind: ExtractionWarningKind::UnsupportedAttribute,
                span: inclusive_span(span),
                message: format!(
                    "Skipping documentation variants with an unsupported attribute: {error}"
                ),
            })
        }))
    }
}

/// A fragment guard or owner constraint with its source span for coverage diagnostics.
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

    fn for_fragment(&self, span: Span) -> SpannedPredicate {
        self.condition.clone().unwrap_or_else(|| SpannedPredicate {
            predicate: Predicate::always(),
            span,
        })
    }

    fn constrain(&self, condition: SpannedPredicate) -> SpannedPredicate {
        let span = self.covering_span(condition.span);
        // A cfg generated by cfg_attr constrains its owner only when the parent guard applies.
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

/// One owner's guarded documentation fragments, before cfg selection and completeness checks.
#[derive(Default)]
struct OwnerDocumentation {
    fragments: Vec<(SpannedPredicate, DocFragment)>,
    constraints: Vec<SpannedPredicate>,
    has_cfg: bool,
    non_reflowable: bool,
    intervening_attribute: bool,
    unsupported: Vec<ExtractionWarning>,
}

impl OwnerDocumentation {
    fn with_constraints(constraints: &[SpannedPredicate]) -> Self {
        Self {
            constraints: constraints.to_vec(),
            has_cfg: !constraints.is_empty(),
            ..Self::default()
        }
    }

    fn append_attribute_tokens(&mut self, source: &str, group: Group) -> Result<()> {
        match syn::parse2::<DocAttribute>(group.stream()) {
            Ok(mut attribute) => {
                if let DocAttribute::Unsupported(warning) = &mut attribute {
                    warning.span = inclusive_span(group.span());
                }
                self.append_attribute(source, attribute, &GuardContext::default())?;
            }
            Err(error) => {
                // An unparsed attribute could supply a Markdown fence. Skip this owner's
                // documentation rather than interpreting nested tokens as independent attributes.
                self.unsupported.push(ExtractionWarning {
                    kind: ExtractionWarningKind::UnsupportedAttribute,
                    span: inclusive_span(group.span()),
                    message: format!(
                        "Skipping documentation block with an unsupported attribute: {error}"
                    ),
                });
            }
        }
        Ok(())
    }

    fn append_attribute(
        &mut self,
        source: &str,
        attribute: DocAttribute,
        guard: &GuardContext,
    ) -> Result<()> {
        match attribute {
            DocAttribute::Doc(assignment) => {
                let Some(fragment) = assignment.into_fragment(source)? else {
                    self.intervening_attribute = true;
                    return Ok(());
                };
                if let Some((_, previous)) = self.fragments.last() {
                    // Structural adjacency can cross syntax that reflow must never replace.
                    let same_variant = match (previous, &fragment) {
                        (DocFragment::Literal(previous), DocFragment::Literal(literal)) => {
                            previous.variant() == literal.variant()
                        }
                        _ => false,
                    };
                    self.non_reflowable |= self.intervening_attribute
                        || fragment.span().start.line != previous.span().end.line + 1
                        || !same_variant;
                }
                match &fragment {
                    DocFragment::Literal(literal) => {
                        if let crate::CommentVariant::MacroDocEqStr(prefix, _) = literal.variant() {
                            // Reflow must not repeat an item or an incomplete attribute wrapper.
                            let prefix =
                                String::from_iter(prefix.chars().filter(|c| !c.is_whitespace()));
                            self.non_reflowable |= !matches!(
                                prefix.as_str(),
                                "#[doc=" | "#![doc=" | "#[r#doc=" | "#![r#doc="
                            );
                        }
                    }
                    DocFragment::UnexpandedMacro(_) | DocFragment::Unsupported(_) => {
                        self.non_reflowable = true;
                    }
                }
                self.intervening_attribute = false;
                self.fragments
                    .push((guard.for_fragment(fragment.span()), fragment));
            }
            DocAttribute::Conditional {
                condition,
                attributes,
            } => {
                self.has_cfg = true;
                let previous_len = self.fragments.len();
                let guard = guard.nested(condition);
                for attribute in attributes {
                    self.append_attribute(source, attribute, &guard)?;
                }
                self.non_reflowable |= self.fragments.len() != previous_len;
                self.intervening_attribute = true;
            }
            DocAttribute::Cfg(condition) => {
                self.has_cfg = true;
                self.constraints.push(guard.constrain(condition));
                self.intervening_attribute = true;
            }
            DocAttribute::Unsupported(warning) => {
                let fragment = DocFragment::Unsupported(warning);
                self.fragments
                    .push((guard.for_fragment(fragment.span()), fragment));
                self.non_reflowable = true;
                self.intervening_attribute = true;
            }
            DocAttribute::Other => self.intervening_attribute = true,
        }
        Ok(())
    }

    /// Return literals only when every selected fragment has known text; otherwise warn and skip.
    fn select_complete_literals(
        &self,
        selected: impl IntoIterator<Item = usize>,
        warnings: &mut Vec<ExtractionWarning>,
        warned_fragments: &mut HashSet<Span>,
    ) -> Option<Vec<TrimmedLiteral>> {
        let mut literals = Vec::new();
        let mut unknown_fragments = Vec::new();
        for index in selected {
            match &self.fragments[index].1 {
                DocFragment::Literal(literal) => literals.push(literal.clone()),
                DocFragment::UnexpandedMacro(span) => unknown_fragments.push(ExtractionWarning {
                    kind: ExtractionWarningKind::OpaqueDocumentation,
                    span: *span,
                    message: "Skipping documentation variants containing an unexpanded documentation macro; its Markdown content is unknown".to_owned(),
                }),
                DocFragment::Unsupported(warning) => unknown_fragments.push(warning.clone()),
            }
        }
        // Unknown content may open or close a fence, even inside known fenced content.
        for warning in &unknown_fragments {
            if warned_fragments.insert(warning.span) {
                warnings.push(warning.clone());
            }
        }
        unknown_fragments.is_empty().then_some(literals)
    }

    /// Project guarded fragments and emit only documentation selections with fully known text.
    fn emit_complete_literal_sets(
        self,
        sets: &mut Vec<LiteralSet>,
        warnings: &mut Vec<ExtractionWarning>,
        budget: CfgBudget,
    ) {
        // Preserve unsupported-attribute diagnostics even when no documentation was parsed.
        if !self.unsupported.is_empty() {
            warnings.extend(self.unsupported);
            return;
        }
        if self.fragments.is_empty() {
            return;
        }
        let mut warned_fragments = HashSet::new();
        // Unconditional documentation needs no symbolic projection or cfg budget.
        if !self.has_cfg {
            if let Some(literals) = self.select_complete_literals(
                0..self.fragments.len(),
                warnings,
                &mut warned_fragments,
            ) {
                sets.push(LiteralSet::from_doc_literals(literals, self.non_reflowable));
            }
            return;
        }
        // Project fragment selections using this owner's explicit cfg budget.
        let report = project(
            self.fragments.iter().map(|(guard, _)| &guard.predicate),
            self.constraints
                .iter()
                .map(|constraint| &constraint.predicate),
            budget,
        );
        // Locate incomplete-coverage warnings at the responsible source condition.
        if let Some(warning) = report.warning {
            let span = match warning.site {
                ProjectionSite::Guard(index) => self.fragments[index].0.span,
                ProjectionSite::Constraint(index) => self.constraints[index].span,
            };
            let complete_count = report
                .projections
                .iter()
                .filter(|projection| {
                    projection
                        .selected
                        .iter()
                        .all(|&index| matches!(self.fragments[index].1, DocFragment::Literal(_)))
                })
                .count();
            let message = format!(
                "incomplete cfg documentation coverage: {}; checked {} complete documentation variants from a bounded sample of configurations",
                warning.error, complete_count,
            );
            let message = if report.projections.is_empty() {
                format!("{message}; no satisfying nonempty documentation variant was found in the sample, which does not prove the documentation unreachable")
            } else {
                message
            };
            warnings.push(ExtractionWarning {
                kind: ExtractionWarningKind::CfgBudget,
                span,
                message,
            });
        }
        // Reject selections containing unknown fragments before constructing literal sets.
        // Known selections retain their original source mappings and symbolic cfg witness.
        for projection in report.projections {
            let Some(literals) =
                self.select_complete_literals(projection.selected, warnings, &mut warned_fragments)
            else {
                continue;
            };
            let mut set = LiteralSet::from_doc_literals(literals, self.non_reflowable);
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
        self.parse_scope(source, stream, OwnerDocumentation::default())
    }

    fn parse_scope(
        &mut self,
        source: &str,
        stream: TokenStream,
        mut owner: OwnerDocumentation,
    ) -> Result<()> {
        let mut tokens = VecDeque::from_iter(stream);
        // Inner and outer docs contribute to the same owner's Markdown, so combine
        // their fragments and constraints before cfg projection and completeness checks.
        while matches!(attribute_prefix(&tokens), Some((AttributeStyle::Inner, _))) {
            let (_, group) = take_attribute(&mut tokens).unwrap();
            owner.append_attribute_tokens(source, group)?;
        }
        let enclosing = owner.constraints.clone();
        owner.emit_complete_literal_sets(&mut self.set, &mut self.warnings, self.budget);

        // Start each sibling with only the enclosing owner's constraints.
        let mut documentation = OwnerDocumentation::with_constraints(&enclosing);
        let mut header = TokenStream::new();
        let mut attribute_style = None;
        while !tokens.is_empty() {
            if let Some((style, group)) = take_attribute(&mut tokens) {
                if !header.is_empty() || attribute_style.is_some_and(|previous| previous != style) {
                    documentation.emit_complete_literal_sets(
                        &mut self.set,
                        &mut self.warnings,
                        self.budget,
                    );
                    documentation = OwnerDocumentation::with_constraints(&enclosing);
                    header = TokenStream::new();
                }
                attribute_style = Some(style);
                documentation.append_attribute_tokens(source, group)?;
                continue;
            }
            let tree = tokens.pop_front().unwrap();
            match tree {
                TokenTree::Group(group)
                    if group.delimiter() == Delimiter::Brace && has_item_body(&header) =>
                {
                    // Body inner attributes constrain both the owner and its descendants.
                    self.parse_scope(source, group.stream(), documentation)?;
                    documentation = OwnerDocumentation::with_constraints(&enclosing);
                    header = TokenStream::new();
                    attribute_style = None;
                }
                TokenTree::Group(group) => {
                    // Retain token-level traversal for expressions and malformed Rust samples.
                    let context = OwnerDocumentation::with_constraints(&documentation.constraints);
                    self.parse_scope(source, group.stream(), context)?;
                    header.extend([TokenTree::Group(group)]);
                }
                TokenTree::Punct(punct) if punct.as_char() == ';' => {
                    documentation.emit_complete_literal_sets(
                        &mut self.set,
                        &mut self.warnings,
                        self.budget,
                    );
                    documentation = OwnerDocumentation::with_constraints(&enclosing);
                    header = TokenStream::new();
                    attribute_style = None;
                }
                tree => header.extend([tree]),
            }
        }
        documentation.emit_complete_literal_sets(&mut self.set, &mut self.warnings, self.budget);
        Ok(())
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

    fn assert_documentation(source: &str, expected_texts: &[&str]) -> Vec<crate::CheckableChunk> {
        let clusters = Clusters::load_from_str(source, true, false).unwrap();
        let expected_warning_count = 0;
        assert_eq!(
            clusters.warnings().len(),
            expected_warning_count,
            "source:\n{source}\nwarnings: {:?}",
            clusters.warnings()
        );
        assert_extracted_documentation(source, clusters, expected_texts)
    }

    fn assert_extracted_documentation(
        source: &str,
        clusters: Clusters,
        expected_texts: &[&str],
    ) -> Vec<crate::CheckableChunk> {
        let chunks = Vec::<crate::CheckableChunk>::from(clusters);
        let mut actual_texts = Vec::from_iter(chunks.iter().map(|chunk| {
            Vec::from_iter(
                chunk
                    .erase_cmark(&Default::default())
                    .as_str()
                    .split_whitespace(),
            )
            .join(" ")
        }));
        let mut expected_texts = Vec::from_iter(expected_texts.iter().map(|text| text.to_string()));
        actual_texts.sort();
        expected_texts.sort();
        assert_eq!(actual_texts, expected_texts, "source:\n{source}");

        // Every selected fragment must still point to its original source, including gaps.
        for chunk in &chunks {
            for (range, span) in chunk.iter().filter(|(range, _)| !range.is_empty()) {
                let expected_content = crate::util::sub_chars(chunk.as_str(), range.clone());
                let actual_content = crate::util::load_span_from(source.as_bytes(), *span).unwrap();
                assert_eq!(actual_content, expected_content);
            }
        }
        chunks
    }

    fn assert_warnings(
        source: &str,
        clusters: &Clusters,
        expected_warnings: &[(ExtractionWarningKind, &str)],
    ) {
        let expected_count = expected_warnings.len();
        assert_eq!(clusters.warnings().len(), expected_count);
        // Warning samples pair a diagnostic kind with its exact source content.
        for (warning, (expected_kind, expected_source)) in
            clusters.warnings().iter().zip(expected_warnings)
        {
            assert_eq!(warning.kind, *expected_kind);
            let actual_source =
                crate::util::load_span_from(source.as_bytes(), warning.span).unwrap();
            assert_eq!(actual_source, *expected_source);
            assert!(!warning.message.is_empty());
        }
    }

    /// Direct and cfg_attr-generated constraints accept one trailing comma, not extra predicates.
    #[test]
    fn cfg_trailing_commas_preserve_documentation_and_predicate_spans() {
        /// An attribute spelling paired with the documentation and witness it must retain.
        struct Case {
            source: &'static str,
            expected_text: &'static str,
            expected_witness: &'static str,
        }
        let cases = [
            Case {
                source: r#"#[cfg(portable,)]
/// Portable packet guide.
struct PacketGuide;"#,
                expected_text: "Portable packet guide.",
                expected_witness: "portable",
            },
            Case {
                source: r#"#[cfg_attr(enabled, cfg(portable,), doc = "Enabled routing.")]
#[cfg(enabled,)]
/// Shared routing.
struct Router;"#,
                expected_text: "Enabled routing. Shared routing.",
                expected_witness: "all(enabled, portable)",
            },
            Case {
                source: r#"#[cfg_attr(enabled, cfg_attr(ready, cfg(portable,), doc = "Portable transport."))]
#[cfg(all(enabled, ready),)]
struct Transport;"#,
                expected_text: "Portable transport.",
                expected_witness: "all(enabled, ready, portable)",
            },
        ];
        for Case {
            source,
            expected_text,
            expected_witness,
        } in cases
        {
            let expected_texts = [expected_text];
            let chunks = assert_documentation(source, &expected_texts);
            let actual_witness = chunks[0].cfg_example();
            let expected_witness = Some(expected_witness);
            assert_eq!(actual_witness, expected_witness);
        }

        let source = r#"#[cfg(portable, extra)]
/// Unsupported constraint.
struct Unsupported;
/// Independent guide.
struct Independent;"#;
        let expected_texts = ["Independent guide."];
        let expected_warnings = [(
            ExtractionWarningKind::UnsupportedAttribute,
            "[cfg(portable, extra)]",
        )];
        let clusters = Clusters::load_from_str(source, true, false).unwrap();
        assert_warnings(source, &clusters, &expected_warnings);
        assert_extracted_documentation(source, clusters, &expected_texts);

        let source = r#"#[cfg(portable,)]
/// Portable packet guide.
struct PacketGuide;"#;
        let budget = CfgBudget {
            variables: 0,
            samples: 2,
            ..CfgBudget::default()
        };
        let expected_warning_count = 1;
        let expected_predicate = "portable";
        let clusters = Clusters::load_from_str_with_budget(source, true, false, budget).unwrap();
        assert_eq!(clusters.warnings().len(), expected_warning_count);
        let actual_predicate =
            crate::util::load_span_from(source.as_bytes(), clusters.warnings()[0].span).unwrap();
        assert_eq!(actual_predicate, expected_predicate);
    }

    /// Compiler-accepted attribute forms retain only reachable, source-backed prose.
    #[test]
    fn attribute_syntax_preserves_reachable_documentation() {
        /// A complete Rust sample paired with its expected documentation owners.
        struct Case {
            /// Source containing attributes whose payloads must not be misinterpreted.
            source: &'static str,
            /// Complete prose selections, excluding discarded attribute expressions.
            expected_texts: &'static [&'static str],
        }
        let cases = [
            Case {
                source: r#"#[cfg_attr(any(), doc = ("Inactive routing."))]
#[cfg_attr(any(), doc = |packet, route| packet)]
/// Shared routing.
struct Router;"#,
                expected_texts: &["Shared routing."],
            },
            Case {
                // Discarded suffixes must never reach source-backed literal loading.
                source: r#"#[cfg_attr(any(), doc = "Inactive export."suffix)]
/// Export policy.
struct ExportPolicy;"#,
                expected_texts: &["Export policy."],
            },
            Case {
                source: r##"#[cfg_attr(all(), cfg_attr(any(), r#doc = r#"Inactive import."#suffix), doc = "Import policy.")]
struct ImportPolicy;"##,
                expected_texts: &["Import policy."],
            },
            Case {
                source: r#"#[r#cfg_attr(all(), cfg_attr(any(), doc = make_packet_docs()), r#doc = "Packet delivery.")]
struct Packet;"#,
                expected_texts: &["Packet delivery."],
            },
            Case {
                source: r#"#![r#cfg_attr(any(), r#doc = CRATE_GUIDE)]
//! Dispatch crate.
#[doc = "Outer dispatch."]
mod dispatch {
    #![cfg_attr(any(), doc = { "Inactive module guide." })]
    //! Inner dispatch.
    /// Queue ownership.
    struct Queue;
}"#,
                expected_texts: &[
                    "Dispatch crate.",
                    "Outer dispatch. Inner dispatch.",
                    "Queue ownership.",
                ],
            },
            Case {
                source: r#"#[cfg_attr(any(), clippy::routing_hint = |packet, cfg()| packet)]
#[cfg_attr(any(), clippy::packet_hint = inspect::<u8, u16>())]
#[cfg_attr(any(), clippy::queue_hint = inspect::<u8, cfg_attr()>() )]
#[cfg_attr(all(), clippy::audit[route, queue], rustfmt::skip{route, queue})]
/// Queue routing guide.
struct Queue;"#,
                expected_texts: &["Queue routing guide."],
            },
            Case {
                source: r#"#[cfg_attr(all(), unsafe(no_mangle), doc = "Foreign dispatch.")]
pub extern "C" fn dispatch() {}"#,
                expected_texts: &["Foreign dispatch."],
            },
            Case {
                source: r#"#[cfg_attr(all(),)]
#[cfg_attr(any(),)]
#[doc = "Outer transport."]
mod transport {
    #![r#cfg_attr(all(), cfg_attr(any(),),)]
    //! Inner transport.
}"#,
                expected_texts: &["Outer transport. Inner transport."],
            },
            Case {
                source: r#"#[cfg_attr(any(), cfg[all()], cfg{all()}, cfg_attr[all(), doc = "Decoy."], cfg_attr{all(), doc = "Decoy."}, cfg, cfg_attr, cfg_attr(all()))]
/// Retained connection guide.
struct Connection;"#,
                expected_texts: &["Retained connection guide."],
            },
            Case {
                source: r#"#[cfg(any())]
#[doc = ("Unreachable session guide.")]
struct Session;
/// Reachable handshake.
struct Handshake;"#,
                expected_texts: &["Reachable handshake."],
            },
            Case {
                source: r#"#[cfg(online)]
mod network {
    #[cfg_attr(not(online), doc = ("Inactive offline guide."))]
    /// Online transport.
    struct Transport;
}"#,
                expected_texts: &["Online transport."],
            },
            Case {
                // rustc 1.99 ignores these with invalid_doc_attributes; 1.88 rejects them.
                source: r#"#[allow(invalid_doc_attributes)]
#[doc = 42]
/// Retry policy.
#[cfg_attr(all(), doc = true, doc = 'R', doc = b'R', doc = b"Retry", doc = 1.25, doc = c"Retry")]
/// Bounded backoff.
struct RetryPolicy;"#,
                expected_texts: &["Retry policy. Bounded backoff."],
            },
            Case {
                source: r##"#[r#cfg(all(),)]
#[r#cfg_attr(all(), r#doc = r#"Encrypted transport."#)]
struct SecureTransport;"##,
                expected_texts: &["Encrypted transport."],
            },
        ];
        for Case {
            source,
            expected_texts,
        } in cases
        {
            assert_documentation(source, expected_texts);
        }
    }

    /// Unsupported expressions and malformed configuration payloads affect selected alternatives only.
    #[test]
    fn unsupported_attribute_payloads_remain_guarded_and_located() {
        /// An unsupported source payload paired with surviving prose and its exact warning span.
        struct Case {
            /// Rust source, including intentionally invalid forms when their guards are active.
            source: &'static str,
            /// Documentation selections that do not include the unsupported payload.
            expected_texts: &'static [&'static str],
            /// Exact source slice identified as an unsupported attribute or doc expression.
            expected_payload: &'static str,
        }
        let cases = [
            Case {
                source: r#"#[doc = ("Not a direct literal.")]
/// Suppressed routing.
struct Router;
/// Independent queue.
struct Queue;"#,
                expected_texts: &["Independent queue."],
                expected_payload: r#"("Not a direct literal.")"#,
            },
            Case {
                source: r#"#[doc = "Invalid export guide."suffix]
/// Suppressed export.
struct Export;
/// Independent manifest.
struct Manifest;"#,
                expected_texts: &["Independent manifest."],
                expected_payload: r#""Invalid export guide."suffix"#,
            },
            Case {
                source: r##"#[cfg_attr(remote, doc = r#"Invalid remote import."#suffix)]
/// Local import.
struct Import;"##,
                expected_texts: &["Local import."],
                expected_payload: r##"r#"Invalid remote import."#suffix"##,
            },
            Case {
                source: r#"#[cfg_attr(remote, doc = ROUTING_DOCS)]
#[cfg_attr(verbose, doc = "Diagnostics.")]
/// Local routing.
struct Router;"#,
                expected_texts: &["Local routing.", "Diagnostics. Local routing."],
                expected_payload: "ROUTING_DOCS",
            },
            Case {
                source: r#"#[cfg_attr(all(), doc = make_transport_docs())]
/// Suppressed transport.
struct Transport;"#,
                expected_texts: &[],
                expected_payload: "make_transport_docs()",
            },
            Case {
                source: r#"#[cfg_attr(remote, doc = {
    "Unknown remote guide."
})]
/// Local session.
struct Session;"#,
                expected_texts: &["Local session."],
                expected_payload: r#"{
    "Unknown remote guide."
}"#,
            },
            Case {
                source: r#"#[cfg_attr(remote, doc = (concat!("Remote", " guide.")))]
/// Local handshake.
struct Handshake;"#,
                expected_texts: &["Local handshake."],
                expected_payload: r#"(concat!("Remote", " guide."))"#,
            },
            Case {
                source: r#"#[cfg_attr(remote, clippy::queue_hint = inspect::<u8, cfg_attr()>() )]
/// Local queue.
struct Queue;"#,
                expected_texts: &["Local queue."],
                expected_payload: "clippy::queue_hint = inspect::<u8, cfg_attr()>()",
            },
            Case {
                source: r#"#[cfg_attr(remote, cfg[all()])]
/// Local connection.
struct Connection;"#,
                expected_texts: &["Local connection."],
                expected_payload: "cfg[all()]",
            },
            Case {
                source: r#"#[cfg_attr(remote, cfg_attr{all(), doc = "Unknown remote guide."})]
/// Local dispatch.
struct Dispatch;"#,
                expected_texts: &["Local dispatch."],
                expected_payload: r#"cfg_attr{all(), doc = "Unknown remote guide."}"#,
            },
            Case {
                source: r#"#[cfg_attr(all(), cfg_attr(all()))]
/// Suppressed packet.
struct Packet;"#,
                expected_texts: &[],
                expected_payload: "cfg_attr(all())",
            },
            Case {
                source: r#"#[cfg{all()}]
/// Suppressed handshake.
struct Handshake;"#,
                expected_texts: &[],
                expected_payload: "[cfg{all()}]",
            },
            Case {
                source: r#"#[cfg_attr[all(), doc = "Unknown encryption guide."]]
/// Suppressed encryption.
struct Encryption;"#,
                expected_texts: &[],
                expected_payload: r#"[cfg_attr[all(), doc = "Unknown encryption guide."]]"#,
            },
            Case {
                source: r#"#[cfg]
/// Suppressed protocol.
struct Protocol;"#,
                expected_texts: &[],
                expected_payload: "[cfg]",
            },
        ];
        for Case {
            source,
            expected_texts,
            expected_payload,
        } in cases
        {
            let expected_warnings = [(
                ExtractionWarningKind::UnsupportedAttribute,
                expected_payload,
            )];
            let clusters = Clusters::load_from_str(source, true, false).unwrap();
            assert_warnings(source, &clusters, &expected_warnings);
            assert_extracted_documentation(source, clusters, expected_texts);
        }
    }

    /// Raw or qualified macro paths and all macro delimiters remain opaque, including nested calls.
    #[test]
    fn doc_macro_expression_syntax_preserves_opaque_spans() {
        /// A macro expression paired with its diagnostic source and independent owner's prose.
        struct Case {
            /// Source containing an unexpanded documentation macro.
            source: &'static str,
            /// Macro invocation slice, without the surrounding doc attribute syntax.
            expected_macro: &'static str,
            /// Complete documentation belonging to an unaffected neighboring item.
            expected_text: &'static str,
        }
        let cases = [
            Case {
                source: r#"#[r#doc = r#concat!("Route", concat!(" guide", "."))]
struct Route;
/// Queue guide.
struct Queue;"#,
                expected_macro: r#"r#concat!("Route", concat!(" guide", "."))"#,
                expected_text: "Queue guide.",
            },
            Case {
                source: r#"#[doc = r#concat!["Packet", " guide."]]
struct Packet;
/// Session guide.
struct Session;"#,
                expected_macro: r#"r#concat!["Packet", " guide."]"#,
                expected_text: "Session guide.",
            },
            Case {
                source: r#"#[doc = ::std::concat!{"Handshake", " guide."}]
struct Handshake;
/// Protocol guide.
struct Protocol;"#,
                expected_macro: r#"::std::concat!{"Handshake", " guide."}"#,
                expected_text: "Protocol guide.",
            },
        ];
        for Case {
            source,
            expected_macro,
            expected_text,
        } in cases
        {
            let expected_texts = [expected_text];
            let expected_warnings = [(ExtractionWarningKind::OpaqueDocumentation, expected_macro)];
            let clusters = Clusters::load_from_str(source, true, false).unwrap();
            assert_warnings(source, &clusters, &expected_warnings);
            assert_extracted_documentation(source, clusters, &expected_texts);
        }
    }

    /// Unsupported expression fragments participate in the same bounded projection as macros.
    #[test]
    fn unsupported_doc_expressions_preserve_cfg_budget_diagnostics() {
        let source = r#"/// Packet guide.
#[cfg_attr(remote, doc = ("Unknown remote guide."))]
struct Packet;"#;
        let budget = CfgBudget {
            variants: 1,
            samples: 4,
            ..CfgBudget::default()
        };
        let expected_texts = ["Packet guide."];
        let expected_warnings = [
            (ExtractionWarningKind::CfgBudget, "remote"),
            (
                ExtractionWarningKind::UnsupportedAttribute,
                r#"("Unknown remote guide.")"#,
            ),
        ];
        let expected_checked_count = "checked 1 complete documentation variants";
        let clusters = Clusters::load_from_str_with_budget(source, true, false, budget).unwrap();
        assert_warnings(source, &clusters, &expected_warnings);
        assert!(clusters.warnings()[0]
            .message
            .contains(expected_checked_count));
        assert_extracted_documentation(source, clusters, &expected_texts);
    }

    /// Module inner fragments close either conditional outer fence before Markdown erasure.
    #[test]
    fn module_outer_and_inner_docs_share_one_markdown_document() {
        let source = r#"/// Transport examples.
///
#[cfg_attr(enabled, doc = "```rust")]
#[cfg_attr(not(enabled), doc = "```ignore")]
mod transport {
    //! let recieve_packet = 1;
    //! ```
    //!
    //! Transport details.
    /// Child route.
    fn route() {}
}"#;
        let expected_texts = [
            "Transport examples. Transport details.",
            "Transport examples. Transport details.",
            "Child route.",
        ];
        let chunks = assert_documentation(source, &expected_texts);
        let expected_module_count = 2;
        let module_chunks = Vec::from_iter(
            chunks
                .iter()
                .filter(|chunk| chunk.as_str().contains("Transport examples.")),
        );
        assert_eq!(module_chunks.len(), expected_module_count);
        let expected_code = "let recieve_packet = 1;";
        let expected_fence_count = 2;
        for chunk in module_chunks {
            assert!(chunk.as_str().contains(expected_code));
            assert_eq!(chunk.as_str().matches("```").count(), expected_fence_count);
            assert!(!chunk.is_reflow_safe());
        }
    }

    /// Crate docs stay separate while a module's outer and inner guards remain correlated.
    #[test]
    fn crate_module_and_child_documentation_keep_their_owners() {
        let source = r#"//! Crate overview.
/// Parent.
#[cfg_attr(a, doc = "Parent option.")]
mod parent {
    //! Inside.
    #![cfg_attr(a, doc = "Inside option.")]
    /// Child.
    #[cfg_attr(a, doc = "Child option.")]
    fn child() {}
}
/// Sibling.
fn sibling() {}"#;
        let expected_texts = [
            "Crate overview.",
            "Parent. Inside.",
            "Parent. Parent option. Inside. Inside option.",
            "Child.",
            "Child. Child option.",
            "Sibling.",
        ];
        assert_documentation(source, &expected_texts);
    }

    /// Descendants inherit direct, generated and inner constraints, never a sibling's constraints.
    #[test]
    fn enclosing_cfg_constraints_exclude_unreachable_children_without_leaking() {
        /// A scope layout paired with its reachable documentation fragments.
        struct Case {
            source: &'static str,
            expected_texts: &'static [&'static str],
        }
        let cases = [
            Case {
                source: r#"#[cfg(enabled)]
mod transport {
    #[cfg_attr(not(enabled), doc = "Unreachable socket.")]
    struct Hidden;
    #[cfg_attr(enabled, doc = "Online socket.")]
    struct Socket;
}
#[cfg_attr(not(enabled), doc = "Offline sibling.")]
struct Outside;"#,
                expected_texts: &["Online socket.", "Offline sibling."],
            },
            Case {
                source: r#"#[cfg(online)]
/// Online module.
mod transport {
    #![cfg(tls)]
    //! Secure module.
    #[cfg(not(tls))]
    /// Unreachable plaintext.
    struct Plaintext;
    /// Secure child.
    struct Secure;
}
#[cfg(not(online))]
/// Offline sibling.
struct Outside;"#,
                expected_texts: &[
                    "Online module. Secure module.",
                    "Secure child.",
                    "Offline sibling.",
                ],
            },
            Case {
                source: r#"#[cfg(online)]
mod transport {
    #[cfg(blocked)]
    /// Blocked child.
    fn blocked() {}
    #[cfg(not(blocked))]
    /// Free child.
    fn free() {}
}"#,
                expected_texts: &["Blocked child.", "Free child."],
            },
            Case {
                source: r#"#[cfg_attr(enabled, cfg(ready))]
mod transport {
    #[cfg(enabled)]
    #[cfg(not(ready))]
    /// Unreachable deployment.
    struct Hidden;
}
/// Independent deployment.
struct Independent;"#,
                expected_texts: &["Independent deployment."],
            },
            Case {
                source: r#"#![cfg(online)]
#[cfg(not(online))]
/// Unreachable crate child.
struct Hidden;
/// Reachable crate child.
struct Visible;"#,
                expected_texts: &["Reachable crate child."],
            },
            Case {
                source: r#"#[cfg(enabled)]
/// Unreachable outer documentation.
mod transport {
    #![cfg(not(enabled))]
    //! Unreachable inner documentation.
    /// Unreachable descendant.
    struct Hidden;
}
/// Independent transport.
struct Independent;"#,
                expected_texts: &["Independent transport."],
            },
            Case {
                source: r#"#[cfg(online)]
/// Packet storage.
struct Packet<const CAPACITY: usize = { 4 }> {
    #[cfg(not(online))]
    /// Unreachable field.
    hidden: (),
    /// Packet payload.
    payload: (),
}
#[cfg(offline)]
struct Offline(
    #[cfg(not(offline))]
    /// Unreachable tuple field.
    (),
    /// Offline payload.
    (),
);"#,
                expected_texts: &["Packet storage.", "Packet payload.", "Offline payload."],
            },
        ];
        for Case {
            source,
            expected_texts,
        } in cases
        {
            assert_documentation(source, expected_texts);
        }
    }

    /// Header recognition must not require valid item bodies or parse decoy payloads as docs.
    #[test]
    fn item_ownership_preserves_tolerant_token_traversal() {
        let source = r#"const TOKENS: () = (doc = "Expression decoy.");
consume!(doc = "Macro decoy.");
#[other(doc = "Attribute decoy.")]
#[cfg(online)]
/// Packet parser.
fn parse_packet() {
    #![cfg(tls)]
    //! Parser details.
    tokens that are not valid Rust;
    #[cfg(not(tls))]
    /// Unreachable parser.
    fn hidden() {}
    /// Nested parser.
    fn helper() {}
}
#[cfg(not(online))]
/// Offline parser.
fn offline() {}"#;
        let expected_texts = [
            "Packet parser. Parser details.",
            "Nested parser.",
            "Offline parser.",
        ];
        let chunks = assert_documentation(source, &expected_texts);
        let expected_witnesses = [
            Some("all(online, tls)"),
            Some("all(online, tls)"),
            Some("not(online)"),
        ];
        let actual_witnesses = Vec::from_iter(chunks.iter().map(|chunk| chunk.cfg_example()));
        assert_eq!(actual_witnesses, expected_witnesses);
    }

    /// Inherited budget failures retain the enclosing predicate's original source span.
    #[test]
    fn inherited_constraints_preserve_budget_warning_locations() {
        let source = r#"#[cfg(all(online, tls),)]
mod transport {
    /// Secure packet guide.
    struct PacketGuide;
}
/// Independent packet guide.
struct Independent;"#;
        let budget = CfgBudget {
            variables: 1,
            samples: 4,
            ..CfgBudget::default()
        };
        let expected_warning_count = 1;
        let expected_predicate = "all(online, tls)";
        let expected_texts = ["Secure packet guide.", "Independent packet guide."];
        let clusters = Clusters::load_from_str_with_budget(source, true, false, budget).unwrap();
        assert_eq!(clusters.warnings().len(), expected_warning_count);
        let actual_predicate =
            crate::util::load_span_from(source.as_bytes(), clusters.warnings()[0].span).unwrap();
        assert_eq!(actual_predicate, expected_predicate);
        let chunks = Vec::<crate::CheckableChunk>::from(clusters);
        let actual_texts = Vec::from_iter(chunks.iter().map(|chunk| chunk.as_str().trim()));
        assert_eq!(actual_texts, expected_texts);
    }

    #[test]
    fn doc_assignment_parse() {
        let _ = syn::parse_str::<DocAssignment>(r########"doc=foo!(bar!(xxx))"########).unwrap();
        let _ = syn::parse_str::<DocAssignment>(r########"doc="s""########).unwrap();
        let _ = syn::parse_str::<DocAssignment>(r########"doc=r#"s"#"########).unwrap();
        let _ = syn::parse_str::<DocAssignment>(r########"doc=r##"s"##"########).unwrap();
        let _ = syn::parse_str::<DocAssignment>(r########"doc=r###"s"###"########).unwrap();
        let _ = syn::parse_str::<DocAssignment>(r########"doc=r####"s"####"########).unwrap();
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
        let mut lengths = Vec::from_iter(clusters.set.iter().map(LiteralSet::len));
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
        let expected_kind = ExtractionWarningKind::CfgBudget;
        let expected_warning_count = 1;
        let clusters = Clusters::load_from_str_with_budget(source, true, false, budget).unwrap();
        assert!(!clusters.set.is_empty());
        assert_eq!(clusters.warnings().len(), expected_warning_count);
        let warning = &clusters.warnings()[0];
        assert_eq!(warning.kind, expected_kind);
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

    /// A macro-supplied fence makes its owner's documentation unknown, not a neighboring item's.
    #[test]
    fn opaque_macro_fence_skips_only_its_documentation_run() {
        let source = r#"mod transport {
    /// Packet examples.
    #[doc = concat!("```", "rust")]
    /// let recieve_packet = 1;
    /// ```
    ///
    /// Packet details.
    struct Packet;
    /// Independent routing guide.
    struct Routing;
}"#;
        let expected_texts = ["Independent routing guide."];
        let expected_warnings = [(
            ExtractionWarningKind::OpaqueDocumentation,
            r#"concat!("```", "rust")"#,
        )];
        let clusters = Clusters::load_from_str(source, true, false).unwrap();
        assert_warnings(source, &clusters, &expected_warnings);
        assert_extracted_documentation(source, clusters, &expected_texts);
    }

    /// Unexpanded macro fragments retain nested guards and warn once across selected variants.
    #[test]
    fn conditional_opaque_docs_keep_complete_inactive_alternatives() {
        let source = r#"#[cfg(available)]
/// Router guide.
#[cfg_attr(remote, cfg_attr(available, doc = include_str!("remote.md")))]
#[cfg_attr(verbose, doc = "Diagnostics.")]
struct Router;"#;
        let expected_texts = ["Router guide.", "Router guide. Diagnostics."];
        let expected_warnings = [(
            ExtractionWarningKind::OpaqueDocumentation,
            r#"include_str!("remote.md")"#,
        )];
        let expected_witnesses = [
            Some("all(available, not(remote), not(verbose))"),
            Some("all(available, not(remote), verbose)"),
        ];
        let clusters = Clusters::load_from_str(source, true, false).unwrap();
        assert_warnings(source, &clusters, &expected_warnings);
        let chunks = assert_extracted_documentation(source, clusters, &expected_texts);
        let actual_witnesses = Vec::from_iter(chunks.iter().map(|chunk| chunk.cfg_example()));
        assert_eq!(actual_witnesses, expected_witnesses);
        assert!(chunks.iter().all(|chunk| !chunk.is_reflow_safe()));
    }

    /// Even a fenced macro could close the fence; macro arguments must remain token decoys.
    #[test]
    fn opaque_docs_inside_known_fences_still_suppress_selected_projections() {
        /// A macro layout paired with its complete prose and exact diagnostic source.
        struct Case {
            source: &'static str,
            expected_texts: &'static [&'static str],
            expected_macro: &'static str,
        }
        let cases = [
            Case {
                source: r#"/// Before.
///
/// ```rust
#[cfg_attr(flag, doc = opaque!(doc = "Decoy.", #[cfg_attr(other, doc = "Another decoy.")]))]
/// let recieve_packet = 1;
/// ```
///
/// After.
struct Packet;"#,
                expected_texts: &["Before. After."],
                expected_macro: r#"opaque!(doc = "Decoy.", #[cfg_attr(other, doc = "Another decoy.")])"#,
            },
            Case {
                source: r#"/// Before.
///
/// ```rust
#[doc = include_str!("example.rs")]
/// let recieve_packet = 1;
/// ```
///
/// After.
struct Packet;"#,
                expected_texts: &[],
                expected_macro: r#"include_str!("example.rs")"#,
            },
            Case {
                source: r#"#[doc = include_str!("handbook.md")]
struct Handbook;"#,
                expected_texts: &[],
                expected_macro: r#"include_str!("handbook.md")"#,
            },
        ];
        for Case {
            source,
            expected_texts,
            expected_macro,
        } in cases
        {
            let expected_warnings = [(ExtractionWarningKind::OpaqueDocumentation, expected_macro)];
            let clusters = Clusters::load_from_str(source, true, false).unwrap();
            assert_warnings(source, &clusters, &expected_warnings);
            assert_extracted_documentation(source, clusters, expected_texts);
        }
    }

    /// Unsupported payloads retain located diagnostics even without any parsed doc fragments.
    #[test]
    fn unsupported_attributes_warn_without_surviving_documentation() {
        /// An unsupported owner paired with independent retained prose and payload spans.
        struct Case {
            source: &'static str,
            expected_texts: &'static [&'static str],
            expected_payloads: &'static [&'static str],
        }
        let cases = [
            Case {
                source: r#"/// Packet examples 🦀.
#[cfg_attr(
    unknown_predicate("1.85"),
    doc = "```rust",
)]
/// let recieve_packet = 1;
/// ```
struct Packet;
/// Independent guide.
struct Independent;"#,
                expected_texts: &["Independent guide."],
                expected_payloads: &[r#"[cfg_attr(
    unknown_predicate("1.85"),
    doc = "```rust",
)]"#],
            },
            Case {
                source: r#"#[cfg(unknown_predicate("1.85"))]
#[cfg_attr(unknown_predicate("1.86"), allow(dead_code))]
struct Packet;"#,
                expected_texts: &[],
                expected_payloads: &[
                    r#"[cfg(unknown_predicate("1.85"))]"#,
                    r#"[cfg_attr(unknown_predicate("1.86"), allow(dead_code))]"#,
                ],
            },
        ];
        for Case {
            source,
            expected_texts,
            expected_payloads,
        } in cases
        {
            let expected_warnings = Vec::from_iter(
                expected_payloads
                    .iter()
                    .map(|payload| (ExtractionWarningKind::UnsupportedAttribute, *payload)),
            );
            let clusters = Clusters::load_from_str(source, true, false).unwrap();
            assert_warnings(source, &clusters, &expected_warnings);
            assert_extracted_documentation(source, clusters, expected_texts);
        }
    }

    /// Unreachable macro fragments cannot make an otherwise known documentation selection unknown.
    #[test]
    fn unreachable_opaque_docs_do_not_warn_or_suppress_complete_prose() {
        /// A constraint layout paired with the reachable documentation it permits.
        struct Case {
            source: &'static str,
            expected_texts: &'static [&'static str],
        }
        let cases = [
            Case {
                source: r#"/// Packet guide.
#[cfg_attr(flag, cfg_attr(not(flag), doc = include_str!("unreachable.md")))]
struct Packet;"#,
                expected_texts: &["Packet guide."],
            },
            Case {
                source: r#"#[cfg(online)]
mod transport {
    /// Online guide.
    #[cfg_attr(not(online), doc = include_str!("offline.md"))]
    struct Packet;
}"#,
                expected_texts: &["Online guide."],
            },
            Case {
                source: r#"#[cfg(false)]
#[doc = include_str!("unreachable.md")]
struct Packet;"#,
                expected_texts: &[],
            },
        ];
        for Case {
            source,
            expected_texts,
        } in cases
        {
            assert_documentation(source, expected_texts);
        }
    }

    /// Equal macro expressions at distinct locations each warn once, not once per projection.
    #[test]
    fn opaque_warning_identity_is_the_macro_source_span() {
        let source = r#"/// Packet guide.
#[cfg_attr(remote, doc = include_str!("details.md"))]
#[cfg_attr(verbose, doc = include_str!("details.md"))]
#[cfg_attr(diagnostics, doc = "Diagnostic guide.")]
struct Packet;"#;
        let expected_texts = ["Packet guide.", "Packet guide. Diagnostic guide."];
        let expected_warnings = [
            (
                ExtractionWarningKind::OpaqueDocumentation,
                r#"include_str!("details.md")"#,
            ),
            (
                ExtractionWarningKind::OpaqueDocumentation,
                r#"include_str!("details.md")"#,
            ),
        ];
        let clusters = Clusters::load_from_str(source, true, false).unwrap();
        assert_warnings(source, &clusters, &expected_warnings);
        let actual_spans = Vec::from_iter(clusters.warnings().iter().map(|warning| warning.span));
        let expected_distinct_spans = 2;
        let actual_distinct_spans = HashSet::<Span>::from_iter(actual_spans).len();
        assert_eq!(actual_distinct_spans, expected_distinct_spans);
        assert_extracted_documentation(source, clusters, &expected_texts);
    }

    /// Unexpanded macro fragments count toward the cfg budget before unknown selections are rejected.
    #[test]
    fn opaque_slots_participate_in_cfg_budget_and_keep_warning_kinds_distinct() {
        let source = r#"/// Packet guide.
#[cfg_attr(remote, doc = include_str!("remote.md"))]
struct Packet;"#;
        let budget = CfgBudget {
            variants: 1,
            samples: 4,
            ..CfgBudget::default()
        };
        let expected_texts = ["Packet guide."];
        let expected_warnings = [
            (ExtractionWarningKind::CfgBudget, "remote"),
            (
                ExtractionWarningKind::OpaqueDocumentation,
                r#"include_str!("remote.md")"#,
            ),
        ];
        let expected_checked_count = "checked 1 complete documentation variants";
        let clusters = Clusters::load_from_str_with_budget(source, true, false, budget).unwrap();
        assert_warnings(source, &clusters, &expected_warnings);
        assert!(clusters.warnings()[0]
            .message
            .contains(expected_checked_count));
        assert_extracted_documentation(source, clusters, &expected_texts);
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
