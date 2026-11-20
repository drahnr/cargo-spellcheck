//! Traverse tokens tolerantly, keeping each owner's documentation and constraints together.

use std::collections::VecDeque;

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};

use super::attributes::SpannedPredicate;
use super::limits::{parsing_limit, ParsingLimit, MAX_PARSER_DEPTH};
use super::owner::OwnerDocumentation;
use super::scope::{completed_group, item_body_scope, OwnerRecovery, ScopeContext};
use super::ExtractionWarning;
use crate::errors::Result;
use crate::{CfgBudget, LiteralSet};

pub(super) fn parse(
    source: &str,
    stream: TokenStream,
    sets: &mut Vec<LiteralSet>,
    warnings: &mut Vec<ExtractionWarning>,
    budget: CfgBudget,
) -> Result<()> {
    Parser {
        source,
        sets,
        warnings,
        budget,
    }
    .parse_scope(
        stream,
        OwnerDocumentation::default(),
        ScopeContext::Other,
        0,
    )
}

struct Parser<'a> {
    source: &'a str,
    sets: &'a mut Vec<LiteralSet>,
    warnings: &'a mut Vec<ExtractionWarning>,
    budget: CfgBudget,
}

/// State that must be reset together whenever traversal reaches the next sibling.
struct PendingOwner {
    documentation: OwnerDocumentation,
    header: TokenStream,
    attribute_style: Option<AttributeStyle>,
    recovery: Option<OwnerRecovery>,
}

impl PendingOwner {
    fn new(enclosing: &[SpannedPredicate]) -> Self {
        Self {
            documentation: OwnerDocumentation::with_constraints(enclosing),
            header: TokenStream::new(),
            attribute_style: None,
            recovery: None,
        }
    }

    fn take_documentation(&mut self, enclosing: &[SpannedPredicate]) -> OwnerDocumentation {
        std::mem::replace(self, Self::new(enclosing)).documentation
    }

    fn reject_complex_header(&mut self, scope: ScopeContext) -> bool {
        let Some(limit) = parsing_limit(&self.header) else {
            return false;
        };
        self.documentation.reject(limit);
        self.recovery = Some(OwnerRecovery::new(scope, &self.header));
        self.header = TokenStream::new();
        true
    }
}

impl Parser<'_> {
    fn emit(&mut self, documentation: OwnerDocumentation) {
        documentation.emit_complete_literal_sets(self.sets, self.warnings, self.budget);
    }

    fn finish_owner(&mut self, pending: &mut PendingOwner, enclosing: &[SpannedPredicate]) {
        self.emit(pending.take_documentation(enclosing));
    }

    fn parse_scope(
        &mut self,
        stream: TokenStream,
        mut owner: OwnerDocumentation,
        scope: ScopeContext,
        depth: usize,
    ) -> Result<()> {
        let mut tokens = VecDeque::from_iter(stream);
        if depth > MAX_PARSER_DEPTH {
            owner.reject(ParsingLimit {
                span: tokens
                    .front()
                    .map(TokenTree::span)
                    .unwrap_or_else(proc_macro2::Span::call_site),
                reason: "scope nesting",
            });
        }
        if owner.parsing_limited() {
            self.emit(owner);
            return Ok(());
        }
        // Inner and outer docs form one Markdown document, including cfg constraints.
        while matches!(attribute_prefix(&tokens), Some((AttributeStyle::Inner, _))) {
            let (_, start, group) = take_attribute(&mut tokens).unwrap();
            owner.append_attribute_tokens(self.source, start, group)?;
        }
        let enclosing = owner.constraints().to_vec();
        let skip_scope = owner.parsing_limited();
        self.emit(owner);
        if skip_scope {
            return Ok(());
        }

        let mut pending = PendingOwner::new(&enclosing);
        while !tokens.is_empty() {
            if let Some(recovery) = &mut pending.recovery {
                let at_attribute =
                    recovery.generic_depth == 0 && attribute_prefix(&tokens).is_some();
                if !at_attribute && !recovery.boundary(&tokens.pop_front().unwrap()) {
                    continue;
                }
                self.finish_owner(&mut pending, &enclosing);
                continue;
            }
            if pending.reject_complex_header(scope) {
                continue;
            }
            if let Some((style, start, group)) = take_attribute(&mut tokens) {
                if !pending.header.is_empty()
                    || pending
                        .attribute_style
                        .is_some_and(|previous| previous != style)
                {
                    self.finish_owner(&mut pending, &enclosing);
                }
                pending.attribute_style = Some(style);
                pending
                    .documentation
                    .append_attribute_tokens(self.source, start, group)?;
                continue;
            }
            match tokens.pop_front().unwrap() {
                TokenTree::Group(group) => {
                    let is_brace = group.delimiter() == Delimiter::Brace;
                    if let Some(body_scope) =
                        is_brace.then(|| item_body_scope(&pending.header)).flatten()
                    {
                        // Body inner attributes constrain the owner and its descendants.
                        let documentation = pending.take_documentation(&enclosing);
                        self.parse_scope(group.stream(), documentation, body_scope, depth + 1)?;
                        continue;
                    }

                    // Also visit expressions and malformed Rust, not just parsed items.
                    let context = pending.documentation.for_children();
                    let nested_scope = scope.nested(&pending.header, group.delimiter());
                    self.parse_scope(group.stream(), context, nested_scope, depth + 1)?;
                    pending
                        .header
                        .extend([TokenTree::Group(completed_group(group, nested_scope))]);
                    if pending.reject_complex_header(scope) {
                        continue;
                    }
                    if is_brace && scope.completes_block(&pending.header, &tokens) {
                        self.finish_owner(&mut pending, &enclosing);
                    }
                }
                TokenTree::Punct(punct)
                    if punct.as_char() == ';'
                        || (punct.as_char() == ','
                            && scope.completes_comma_owner(&pending.header)) =>
                {
                    self.finish_owner(&mut pending, &enclosing);
                }
                tree => pending.header.extend([tree]),
            }
        }
        self.emit(pending.documentation);
        Ok(())
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

fn take_attribute(
    tokens: &mut VecDeque<TokenTree>,
) -> Option<(AttributeStyle, proc_macro2::Span, Group)> {
    let (style, group_index) = attribute_prefix(tokens)?;
    let start = tokens.front()?.span();
    let Some(TokenTree::Group(group)) = tokens.get(group_index) else {
        unreachable!("an attribute prefix ends in a bracket group");
    };
    let group = group.clone();
    tokens.drain(..=group_index);
    Some((style, start, group))
}
