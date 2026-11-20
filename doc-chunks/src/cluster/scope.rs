//! Recognize owner boundaries without requiring a complete, valid Rust syntax tree.

use std::collections::VecDeque;

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use syn::parse::Parser;

/// The sibling boundaries recognized in a token group's contents.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ScopeContext {
    Other,
    Variants,
    NamedFields,
    TupleFields,
    Parameters,
    LiteralFields,
    Expressions,
    MatchArms,
    Statements,
}

impl ScopeContext {
    /// Classify a non-item group using its enclosing owner's grammar.
    pub(super) fn nested(self, header: &TokenStream, delimiter: Delimiter) -> Self {
        field_group_scope(header, delimiter, self)
            .or_else(|| is_parameter_group(header, delimiter).then_some(Self::Parameters))
            .or_else(|| expression_group_scope(header, delimiter, self))
            .unwrap_or_else(|| match delimiter {
                Delimiter::Brace if is_match_body(header) => Self::MatchArms,
                Delimiter::Brace => Self::Statements,
                _ => Self::Other,
            })
    }

    pub(super) fn completes_block(
        self,
        header: &TokenStream,
        remaining: &VecDeque<TokenTree>,
    ) -> bool {
        match self {
            Self::Statements => completes_block_statement(header, remaining),
            Self::MatchArms => completes_block_arm(header, remaining),
            _ => false,
        }
    }

    /// Accept a separator only after a complete owner, never inside its generic arguments.
    pub(super) fn completes_comma_owner(self, header: &TokenStream) -> bool {
        match self {
            Self::Variants => syn::parse2::<syn::Variant>(header.clone()).is_ok(),
            Self::NamedFields => syn::Field::parse_named.parse2(header.clone()).is_ok(),
            Self::TupleFields => syn::Field::parse_unnamed.parse2(header.clone()).is_ok(),
            Self::Parameters => syn::parse2::<syn::FnArg>(header.clone()).is_ok(),
            Self::LiteralFields => syn::parse2::<syn::FieldValue>(header.clone()).is_ok(),
            Self::Expressions => syn::parse2::<syn::Expr>(header.clone()).is_ok(),
            Self::MatchArms => {
                let mut candidate = header.clone();
                candidate.extend([TokenTree::Punct(proc_macro2::Punct::new(
                    ',',
                    proc_macro2::Spacing::Alone,
                ))]);
                syn::parse2::<syn::Arm>(candidate).is_ok()
            }
            Self::Other | Self::Statements => false,
        }
    }
}

/// Recovery boundaries for a rejected header, retaining unclosed generic arguments.
pub(super) struct OwnerRecovery {
    comma_boundary: bool,
    type_arguments: bool,
    pub(super) generic_depth: usize,
    previous_punctuation: Option<char>,
    preceding_punctuation: Option<char>,
}

impl OwnerRecovery {
    /// Inspect only the current token level; groups keep their own commas and attributes.
    pub(super) fn new(scope: ScopeContext, header: &TokenStream) -> Self {
        let typed_member = matches!(
            scope,
            ScopeContext::NamedFields | ScopeContext::TupleFields | ScopeContext::Parameters
        );
        let declaration = header.clone().into_iter().any(|tree| {
            matches!(tree, TokenTree::Ident(ident) if matches!(ident.to_string().as_str(), "fn" | "struct" | "enum" | "union" | "trait" | "impl"))
        });
        let mut recovery = Self {
            comma_boundary: typed_member || scope == ScopeContext::Variants,
            type_arguments: typed_member || declaration,
            generic_depth: 0,
            previous_punctuation: None,
            preceding_punctuation: None,
        };
        for tree in header.clone() {
            recovery.observe(&tree);
        }
        recovery
    }

    /// Track type arguments and expression turbofish without interpreting grouped const expressions.
    fn observe(&mut self, tree: &TokenTree) {
        let punctuation = match tree {
            TokenTree::Punct(punct) => Some(punct.as_char()),
            _ => None,
        };
        match punctuation {
            Some('<')
                if self.type_arguments
                    || self.generic_depth > 0
                    || (self.previous_punctuation == Some(':')
                        && self.preceding_punctuation == Some(':'))
                    || self.previous_punctuation == Some('=') =>
            {
                self.generic_depth += 1
            }
            Some('>') if self.previous_punctuation != Some('-') => {
                self.generic_depth = self.generic_depth.saturating_sub(1);
            }
            _ => {}
        }
        self.preceding_punctuation = self.previous_punctuation;
        self.previous_punctuation = punctuation;
    }

    /// Resume at an owner separator, not at a comma still inside a generic argument list.
    pub(super) fn boundary(&mut self, tree: &TokenTree) -> bool {
        if matches!(tree, TokenTree::Punct(punct) if punct.as_char() == ';'
            || (punct.as_char() == ',' && self.comma_boundary && self.generic_depth == 0))
        {
            return true;
        }
        self.observe(tree);
        false
    }
}

/// Retain only syntax needed by later classifiers after a group's contents have been traversed.
pub(super) fn completed_group(group: Group, scope: ScopeContext) -> Group {
    if scope == ScopeContext::Other {
        return group;
    }
    let mut summary = Group::new(group.delimiter(), TokenStream::new());
    summary.set_span(group.span());
    summary
}

/// Recognize an item's body without requiring its contents or the file to parse as Rust.
pub(super) fn item_body_scope(header: &TokenStream) -> Option<ScopeContext> {
    let mut candidate = header.clone();
    candidate.extend([TokenTree::Group(Group::new(
        Delimiter::Brace,
        TokenStream::new(),
    ))]);
    match syn::parse2::<syn::Item>(candidate) {
        Ok(syn::Item::Fn(_)) => Some(ScopeContext::Statements),
        Ok(syn::Item::Enum(_)) => Some(ScopeContext::Variants),
        Ok(syn::Item::Struct(_) | syn::Item::Union(_)) => Some(ScopeContext::NamedFields),
        Ok(
            syn::Item::Mod(_)
            | syn::Item::Impl(_)
            | syn::Item::Trait(_)
            | syn::Item::ForeignMod(_)
            | syn::Item::Macro(_),
        ) => Some(ScopeContext::Other),
        _ => None,
    }
}

/// Identify struct and variant field groups without mistaking header expression groups for fields.
fn field_group_scope(
    header: &TokenStream,
    delimiter: Delimiter,
    scope: ScopeContext,
) -> Option<ScopeContext> {
    let field_scope = match delimiter {
        Delimiter::Brace if scope == ScopeContext::Variants => ScopeContext::NamedFields,
        Delimiter::Parenthesis => ScopeContext::TupleFields,
        _ => return None,
    };
    let mut candidate = header.clone();
    candidate.extend([TokenTree::Group(Group::new(delimiter, TokenStream::new()))]);
    if scope == ScopeContext::Variants {
        let variant = syn::parse2::<syn::Variant>(candidate).ok()?;
        return match variant.fields {
            syn::Fields::Named(_) | syn::Fields::Unnamed(_) => Some(field_scope),
            syn::Fields::Unit => None,
        };
    }
    if delimiter != Delimiter::Parenthesis {
        return None;
    }
    candidate.extend([TokenTree::Punct(proc_macro2::Punct::new(
        ';',
        proc_macro2::Spacing::Alone,
    ))]);
    let item = syn::parse2::<syn::ItemStruct>(candidate).ok()?;
    matches!(item.fields, syn::Fields::Unnamed(_)).then_some(field_scope)
}

/// Recognize function parameter groups without treating function-pointer types as signatures.
fn is_parameter_group(header: &TokenStream, delimiter: Delimiter) -> bool {
    if delimiter != Delimiter::Parenthesis {
        return false;
    }
    let mut candidate = header.clone();
    candidate.extend([TokenTree::Group(Group::new(delimiter, TokenStream::new()))]);
    (|input: syn::parse::ParseStream<'_>| {
        input.parse::<syn::Visibility>()?;
        input.parse::<syn::Signature>()
    })
    .parse2(candidate)
    .is_ok()
}

/// Parse an expression prefix using its enclosing owner's grammar, not arbitrary token suffixes.
fn owner_expression(candidate: TokenStream, scope: ScopeContext) -> Option<syn::Expr> {
    match scope {
        ScopeContext::LiteralFields => syn::parse2::<syn::FieldValue>(candidate)
            .ok()
            .map(|field| field.expr),
        ScopeContext::Variants => syn::parse2::<syn::Variant>(candidate)
            .ok()
            .and_then(|variant| variant.discriminant.map(|(_, expression)| expression)),
        ScopeContext::MatchArms => {
            let mut candidate = candidate;
            candidate.extend([TokenTree::Punct(proc_macro2::Punct::new(
                ',',
                proc_macro2::Spacing::Alone,
            ))]);
            syn::parse2::<syn::Arm>(candidate).ok().map(|arm| *arm.body)
        }
        ScopeContext::Other | ScopeContext::Statements | ScopeContext::Expressions => {
            if let Ok(expression) = syn::parse2::<syn::Expr>(candidate.clone()) {
                return Some(expression);
            }
            if scope == ScopeContext::Expressions {
                return None;
            }
            let mut condition = candidate.clone();
            condition.extend([TokenTree::Group(Group::new(
                Delimiter::Brace,
                TokenStream::new(),
            ))]);
            if let Ok(expression @ (syn::Expr::If(_) | syn::Expr::While(_))) =
                syn::parse2::<syn::Expr>(condition)
            {
                return Some(expression);
            }
            let mut candidate = candidate;
            candidate.extend([TokenTree::Punct(proc_macro2::Punct::new(
                ';',
                proc_macro2::Spacing::Alone,
            ))]);
            match syn::parse2::<syn::Stmt>(candidate).ok()? {
                syn::Stmt::Local(local) => local.init.map(|init| *init.expr),
                syn::Stmt::Expr(expression, _) => Some(expression),
                syn::Stmt::Item(syn::Item::Const(item)) => Some(*item.expr),
                syn::Stmt::Item(syn::Item::Static(item)) => Some(*item.expr),
                _ => None,
            }
        }
        ScopeContext::NamedFields | ScopeContext::TupleFields | ScopeContext::Parameters => None,
    }
}

/// Classify only expression-owned groups; type groups and opaque macro payloads remain unclassified.
fn expression_group_scope(
    header: &TokenStream,
    delimiter: Delimiter,
    scope: ScopeContext,
) -> Option<ScopeContext> {
    let mut candidate = header.clone();
    candidate.extend([TokenTree::Group(Group::new(delimiter, TokenStream::new()))]);
    let expression = owner_expression(candidate, scope)?;
    let mut trailing = &expression;
    loop {
        match trailing {
            syn::Expr::Tuple(_)
            | syn::Expr::Paren(_)
            | syn::Expr::Call(_)
            | syn::Expr::MethodCall(_)
                if delimiter == Delimiter::Parenthesis =>
            {
                return Some(ScopeContext::Expressions)
            }
            syn::Expr::Array(_) | syn::Expr::Repeat(_) if delimiter == Delimiter::Bracket => {
                return Some(ScopeContext::Expressions);
            }
            syn::Expr::Struct(_) if delimiter == Delimiter::Brace => {
                return Some(ScopeContext::LiteralFields);
            }
            syn::Expr::If(expression) => trailing = &expression.cond,
            syn::Expr::While(expression) => trailing = &expression.cond,
            syn::Expr::Closure(expression) => trailing = &expression.body,
            syn::Expr::Let(expression) => trailing = &expression.expr,
            syn::Expr::Assign(expression) => trailing = &expression.right,
            syn::Expr::Binary(expression) => trailing = &expression.right,
            syn::Expr::Reference(expression) => trailing = &expression.expr,
            syn::Expr::Unary(expression) => trailing = &expression.expr,
            syn::Expr::Return(expression) => trailing = expression.expr.as_deref()?,
            syn::Expr::Break(expression) => trailing = expression.expr.as_deref()?,
            _ => return None,
        }
    }
}

/// Recognize a match body behind a statement prefix without parsing the surrounding expression.
fn is_match_body(header: &TokenStream) -> bool {
    let trees = Vec::from_iter(header.clone());
    trees.iter().enumerate().any(|(index, tree)| {
        if !matches!(tree, TokenTree::Ident(ident) if ident == "match") {
            return false;
        }
        let mut candidate = TokenStream::from_iter(trees[index..].iter().cloned());
        candidate.extend([TokenTree::Group(Group::new(
            Delimiter::Brace,
            TokenStream::new(),
        ))]);
        matches!(syn::parse2::<syn::Expr>(candidate), Ok(syn::Expr::Match(_)))
    })
}

/// Keep expression suffixes and else branches attached to their current statement or arm.
fn has_expression_continuation(remaining: &VecDeque<TokenTree>) -> bool {
    match remaining.front() {
        Some(TokenTree::Ident(ident)) if ident == "else" => true,
        Some(TokenTree::Punct(punct)) if matches!(punct.as_char(), '.' | '?') => true,
        Some(TokenTree::Group(group)) => matches!(
            group.delimiter(),
            Delimiter::Parenthesis | Delimiter::Bracket
        ),
        _ => false,
    }
}

/// Recognize a completed block statement, excluding continuations of the same expression.
fn completes_block_statement(header: &TokenStream, remaining: &VecDeque<TokenTree>) -> bool {
    if has_expression_continuation(remaining) {
        return false;
    }
    matches!(
        syn::Block::parse_within.parse2(header.clone()).as_deref(),
        Ok([syn::Stmt::Expr(_, None)])
    )
}

/// Only block-form arm bodies may end without a comma; a parsed prefix alone is insufficient.
fn completes_block_arm(header: &TokenStream, remaining: &VecDeque<TokenTree>) -> bool {
    if has_expression_continuation(remaining) {
        return false;
    }
    let Ok(arm) = syn::parse2::<syn::Arm>(header.clone()) else {
        return false;
    };
    matches!(
        arm.body.as_ref(),
        syn::Expr::Block(_)
            | syn::Expr::Async(_)
            | syn::Expr::Const(_)
            | syn::Expr::Unsafe(_)
            | syn::Expr::Loop(_)
            | syn::Expr::While(_)
            | syn::Expr::ForLoop(_)
            | syn::Expr::If(_)
            | syn::Expr::Match(_)
            | syn::Expr::TryBlock(_)
    )
}
