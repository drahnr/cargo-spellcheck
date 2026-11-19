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

/// Maximum delimiter and traversal nesting admitted to recursive parsing on a small stack.
const MAX_PARSER_DEPTH: usize = 32;
/// Maximum token count admitted to a single classifier or attribute parser invocation.
const MAX_PARSER_TOKENS: usize = 256;
/// Maximum combined nesting and recursive operator density admitted to a parser invocation.
const MAX_PARSER_RECURSION: usize = 32;

/// A source-located reason to skip an owner before invoking a recursive parser.
struct ParsingLimit {
    span: proc_macro2::Span,
    reason: &'static str,
}

impl ParsingLimit {
    /// Reuse the existing unsupported-attribute diagnostic for safely skipped owners.
    fn warning(&self) -> ExtractionWarning {
        ExtractionWarning {
            kind: ExtractionWarningKind::UnsupportedAttribute,
            span: inclusive_span(self.span),
            message: format!(
                "Skipping documentation owner: parser classification complexity limit exceeded ({})",
                self.reason,
            ),
        }
    }
}

/// Inspect tokens iteratively before syn constructs its recursive TokenBuffer or expression AST.
fn parsing_limit(stream: &TokenStream) -> Option<ParsingLimit> {
    parsing_limit_inner(stream, false)
}

/// Recognize flat atom lists whose width does not add parser recursion or delimiter depth.
fn flat_cfg_atoms(stream: TokenStream) -> bool {
    let mut tokens = stream.into_iter().peekable();
    while let Some(tree) = tokens.next() {
        if !matches!(tree, TokenTree::Ident(_)) {
            return false;
        }
        if matches!(tokens.peek(), Some(TokenTree::Punct(punct)) if punct.as_char() == '=') {
            tokens.next();
            if !matches!(tokens.next(), Some(TokenTree::Literal(_))) {
                return false;
            }
        }
        match tokens.next() {
            Some(TokenTree::Punct(punct)) if punct.as_char() == ',' => {}
            None => return true,
            _ => return false,
        }
    }
    true
}

/// Leave flat cfg width to the symbolic budget while retaining limits on nested or opaque syntax.
fn attribute_parsing_limit(stream: &TokenStream) -> Option<ParsingLimit> {
    let flat_cfg = matches!(
        stream.clone().into_iter().next(),
        Some(TokenTree::Ident(ident))
            if matches!(ident.to_string().trim_start_matches("r#"), "cfg" | "cfg_attr")
    );
    parsing_limit_inner(stream, flat_cfg)
}

/// Count recursive work without charging validated flat cfg atom lists as recursive syntax.
fn parsing_limit_inner(stream: &TokenStream, flat_cfg: bool) -> Option<ParsingLimit> {
    let mut pending = vec![(stream.clone().into_iter().peekable(), 0)];
    let mut token_count = 0;
    let mut recursive_tokens = 0;
    while let Some((tokens, depth)) = pending.last_mut() {
        let Some(tree) = tokens.next() else {
            pending.pop();
            continue;
        };
        token_count += 1;
        let recursive = match &tree {
            TokenTree::Punct(punct) => !matches!(punct.as_char(), ',' | ';' | '#' | ':' | '\''),
            TokenTree::Ident(ident) => matches!(
                ident.to_string().as_str(),
                "if" | "while"
                    | "match"
                    | "loop"
                    | "for"
                    | "move"
                    | "async"
                    | "unsafe"
                    | "const"
                    | "let"
                    | "return"
                    | "break"
                    | "yield"
                    | "as"
            ),
            _ => false,
        };
        recursive_tokens += usize::from(recursive);
        let reason = if token_count > MAX_PARSER_TOKENS {
            Some("token count")
        } else if *depth > MAX_PARSER_DEPTH {
            Some("delimiter nesting")
        } else if *depth + recursive_tokens > MAX_PARSER_RECURSION {
            Some("recursive syntax density")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Some(ParsingLimit {
                span: tree.span(),
                reason,
            });
        }
        if flat_cfg
            && matches!(&tree, TokenTree::Ident(ident) if matches!(ident.to_string().trim_start_matches("r#"), "all" | "any"))
        {
            if let Some(TokenTree::Group(group)) = tokens.peek() {
                if group.delimiter() == Delimiter::Parenthesis && flat_cfg_atoms(group.stream()) {
                    if *depth + 1 > MAX_PARSER_DEPTH {
                        return Some(ParsingLimit {
                            span: group.span(),
                            reason: "delimiter nesting",
                        });
                    }
                    tokens.next();
                    continue;
                }
            }
        }
        if let TokenTree::Group(group) = tree {
            let nested_depth = *depth + 1;
            pending.push((group.stream().into_iter().peekable(), nested_depth));
        }
    }
    None
}

/// The sibling boundaries recognized in a token group's contents.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeContext {
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
    /// Accept a separator only after a complete owner, never inside its generic arguments.
    fn completes_comma_owner(self, header: &TokenStream) -> bool {
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
struct OwnerRecovery {
    comma_boundary: bool,
    type_arguments: bool,
    generic_depth: usize,
    previous_punctuation: Option<char>,
    preceding_punctuation: Option<char>,
}

impl OwnerRecovery {
    /// Inspect only the current token level; groups keep their own commas and attributes.
    fn new(scope: ScopeContext, header: &TokenStream) -> Self {
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
    fn boundary(&mut self, tree: &TokenTree) -> bool {
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
fn completed_group(group: Group, scope: ScopeContext) -> Group {
    if scope == ScopeContext::Other {
        return group;
    }
    let mut summary = Group::new(group.delimiter(), TokenStream::new());
    summary.set_span(group.span());
    summary
}

/// Recognize an item's body without requiring its contents or the file to parse as Rust.
fn item_body_scope(header: &TokenStream) -> Option<ScopeContext> {
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

/// One owner's guarded documentation fragments, before cfg selection and completeness checks.
#[derive(Default)]
struct OwnerDocumentation {
    fragments: Vec<(SpannedPredicate, DocFragment)>,
    constraints: Vec<SpannedPredicate>,
    has_cfg: bool,
    non_reflowable: bool,
    intervening_attribute: bool,
    unsupported: Vec<ExtractionWarning>,
    parsing_limited: bool,
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
        if let Some(limit) = attribute_parsing_limit(&group.stream()) {
            let mut warning = limit.warning();
            warning.span = inclusive_span(group.span());
            if limit.reason == "delimiter nesting"
                && matches!(group.stream().into_iter().next(), Some(TokenTree::Ident(ident)) if ident == "cfg" || ident == "cfg_attr")
            {
                warning
                    .message
                    .push_str("; cfg predicate nesting limit exceeded before attribute parsing");
            }
            self.unsupported.push(warning);
            self.parsing_limited = true;
            return Ok(());
        }
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
        self.parse_scope(
            source,
            stream,
            OwnerDocumentation::default(),
            ScopeContext::Other,
            0,
        )
    }

    fn parse_scope(
        &mut self,
        source: &str,
        stream: TokenStream,
        mut owner: OwnerDocumentation,
        scope: ScopeContext,
        depth: usize,
    ) -> Result<()> {
        let mut tokens = VecDeque::from_iter(stream);
        if depth > MAX_PARSER_DEPTH {
            let limit = ParsingLimit {
                span: tokens
                    .front()
                    .map(TokenTree::span)
                    .unwrap_or_else(proc_macro2::Span::call_site),
                reason: "scope nesting",
            };
            owner.unsupported.push(limit.warning());
            owner.parsing_limited = true;
        }
        if owner.parsing_limited {
            owner.emit_complete_literal_sets(&mut self.set, &mut self.warnings, self.budget);
            return Ok(());
        }
        // Inner and outer docs contribute to the same owner's Markdown, so combine
        // their fragments and constraints before cfg projection and completeness checks.
        while matches!(attribute_prefix(&tokens), Some((AttributeStyle::Inner, _))) {
            let (_, group) = take_attribute(&mut tokens).unwrap();
            owner.append_attribute_tokens(source, group)?;
        }
        let enclosing = owner.constraints.clone();
        let skip_scope = owner.parsing_limited;
        owner.emit_complete_literal_sets(&mut self.set, &mut self.warnings, self.budget);
        if skip_scope {
            return Ok(());
        }

        // Start each sibling with only the enclosing owner's constraints.
        let mut documentation = OwnerDocumentation::with_constraints(&enclosing);
        let mut header = TokenStream::new();
        let mut attribute_style = None;
        let mut recovery: Option<OwnerRecovery> = None;
        while !tokens.is_empty() {
            if let Some(owner_recovery) = &mut recovery {
                let at_attribute =
                    owner_recovery.generic_depth == 0 && attribute_prefix(&tokens).is_some();
                if !at_attribute && !owner_recovery.boundary(&tokens.pop_front().unwrap()) {
                    continue;
                }
                documentation.emit_complete_literal_sets(
                    &mut self.set,
                    &mut self.warnings,
                    self.budget,
                );
                documentation = OwnerDocumentation::with_constraints(&enclosing);
                header = TokenStream::new();
                attribute_style = None;
                recovery = None;
                continue;
            }
            if let Some(limit) = parsing_limit(&header) {
                documentation.unsupported.push(limit.warning());
                documentation.parsing_limited = true;
                recovery = Some(OwnerRecovery::new(scope, &header));
                header = TokenStream::new();
                continue;
            }
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
            let body_scope = match &tree {
                TokenTree::Group(group) if group.delimiter() == Delimiter::Brace => {
                    item_body_scope(&header)
                }
                _ => None,
            };
            match tree {
                TokenTree::Group(group) if body_scope.is_some() => {
                    // Body inner attributes constrain both the owner and its descendants.
                    self.parse_scope(
                        source,
                        group.stream(),
                        documentation,
                        body_scope.unwrap(),
                        depth + 1,
                    )?;
                    documentation = OwnerDocumentation::with_constraints(&enclosing);
                    header = TokenStream::new();
                    attribute_style = None;
                }
                TokenTree::Group(group) => {
                    // Retain token-level traversal for expressions and malformed Rust samples.
                    let is_brace = group.delimiter() == Delimiter::Brace;
                    let mut context =
                        OwnerDocumentation::with_constraints(&documentation.constraints);
                    context.parsing_limited = documentation.parsing_limited;
                    let nested_scope = field_group_scope(&header, group.delimiter(), scope)
                        .or_else(|| {
                            is_parameter_group(&header, group.delimiter())
                                .then_some(ScopeContext::Parameters)
                        })
                        .or_else(|| expression_group_scope(&header, group.delimiter(), scope))
                        .unwrap_or_else(|| {
                            if is_brace && is_match_body(&header) {
                                ScopeContext::MatchArms
                            } else if is_brace {
                                ScopeContext::Statements
                            } else {
                                ScopeContext::Other
                            }
                        });
                    self.parse_scope(source, group.stream(), context, nested_scope, depth + 1)?;
                    header.extend([TokenTree::Group(completed_group(group, nested_scope))]);
                    if let Some(limit) = parsing_limit(&header) {
                        documentation.unsupported.push(limit.warning());
                        documentation.parsing_limited = true;
                        recovery = Some(OwnerRecovery::new(scope, &header));
                        header = TokenStream::new();
                        continue;
                    }
                    if is_brace
                        && ((scope == ScopeContext::Statements
                            && completes_block_statement(&header, &tokens))
                            || (scope == ScopeContext::MatchArms
                                && completes_block_arm(&header, &tokens)))
                    {
                        documentation.emit_complete_literal_sets(
                            &mut self.set,
                            &mut self.warnings,
                            self.budget,
                        );
                        documentation = OwnerDocumentation::with_constraints(&enclosing);
                        header = TokenStream::new();
                        attribute_style = None;
                    }
                }
                TokenTree::Punct(punct)
                    if punct.as_char() == ';'
                        || (punct.as_char() == ',' && scope.completes_comma_owner(&header)) =>
                {
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

    /// Variant commas end pending constraints without discarding the enum's own constraints.
    #[test]
    fn enum_variant_constraints_do_not_leak_to_following_payloads() {
        /// An enum layout paired with the documentation reachable after its hidden variant.
        struct Case {
            source: &'static str,
            expected_texts: &'static [&'static str],
        }
        let cases = [
            Case {
                source: r#"enum Packet {
    #[cfg(any())]
    Hidden,
    Active {
        /// Visible payload.
        payload: (),
    },
}"#,
                expected_texts: &["Visible payload."],
            },
            Case {
                source: r#"enum Response {
    #[cfg(any())]
    Cached(usize),
    Fresh(
        /// Fresh response body.
        (),
    ),
}"#,
                expected_texts: &["Fresh response body."],
            },
            Case {
                source: r#"enum Opcode {
    #[cfg(any())]
    Reserved = {
        /// Invisible opcode helper.
        struct ReservedCode;
        0
    },
    Ready {
        /// Ready opcode body.
        body: (),
    },
}"#,
                expected_texts: &["Ready opcode body."],
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

    /// A named field's guard ends before undocumented sibling types are traversed.
    #[test]
    fn named_field_constraints_do_not_leak_to_sibling_const_groups() {
        let source = r#"struct State {
    #[cfg(any())]
    hidden: (),
    visible: [(); {
        /// Visible helper.
        struct Visible;
        0
    }],
}"#;
        let expected_texts = ["Visible helper."];
        syn::parse_file(source).expect("the named-field regression is valid Rust syntax");
        assert_documentation(source, &expected_texts);
    }

    /// Tuple fields release their own guards while retaining the struct's guard.
    #[test]
    fn tuple_field_constraints_do_not_leak_to_sibling_const_groups() {
        let source = r#"#[cfg(storage)]
pub struct Dimensions(
    #[cfg(any())] pub (),
    pub [(); {
        /// Visible dimension helper.
        struct Dimension;
        #[cfg(not(storage))]
        /// Unreachable dimension helper.
        struct UnreachableDimension;
        0
    }],
);"#;
        let expected_texts = ["Visible dimension helper."];
        syn::parse_file(source).expect("the tuple-field regression is valid Rust syntax");
        assert_documentation(source, &expected_texts);
    }

    /// Named and tuple variant fields keep enum and variant guards, not sibling guards.
    #[test]
    fn variant_field_constraints_do_not_leak_to_sibling_const_groups() {
        /// A variant field layout paired with its independently reachable nested documentation.
        struct Case {
            source: &'static str,
            expected_texts: &'static [&'static str],
        }
        let cases = [
            Case {
                source: r#"#[cfg(transport)]
enum Frame {
    #[cfg(ready)]
    Named {
        #[cfg(any())]
        hidden: (),
        visible: [(); {
            /// Visible frame helper.
            struct FrameHelper;
            #[cfg(not(ready))]
            /// Unreachable frame helper.
            struct UnreachableFrame;
            0
        }],
    },
}"#,
                expected_texts: &["Visible frame helper."],
            },
            Case {
                source: r#"#[cfg(transport)]
enum Message {
    #[cfg(ready)]
    Tuple(
        #[cfg(any())] (),
        [(); {
            /// Visible message helper.
            struct MessageHelper;
            #[cfg(not(transport))]
            /// Unreachable message helper.
            struct UnreachableMessage;
            0
        }],
    ),
}"#,
                expected_texts: &["Visible message helper."],
            },
        ];
        for Case {
            source,
            expected_texts,
        } in cases
        {
            syn::parse_file(source).expect("the variant-field regression is valid Rust syntax");
            assert_documentation(source, expected_texts);
        }
    }

    /// A match-arm comma releases its guard before traversing the next arm's body.
    #[test]
    fn match_arm_constraints_do_not_leak_to_sibling_const_groups() {
        let source = r#"fn classify(code: u8) {
    let _result = match code {
        #[cfg(any())]
        0 => (),
        _ => {
            /// Visible classification helper.
            struct Classification;
        },
    };
}"#;
        let expected_texts = ["Visible classification helper."];
        syn::parse_file(source).expect("the match-arm regression is valid Rust syntax");
        assert_documentation(source, &expected_texts);
    }

    /// Field and arm separators must not consume commas inside generic arguments.
    #[test]
    fn field_and_match_arm_generic_commas_preserve_their_owners() {
        /// A guarded generic layout paired with documentation from the independent sibling.
        struct Case {
            source: &'static str,
            expected_texts: &'static [&'static str],
        }
        let cases = [
            Case {
                source: r#"struct Buffers {
    #[cfg(any())]
    hidden: Result<[(); {
        /// Hidden request.
        struct Request; 0
    }], [(); {
        /// Hidden response.
        struct Response; 0
    }]>,
    visible: [(); {
        /// Active buffer.
        struct Buffer; 0
    }],
}"#,
                expected_texts: &["Active buffer."],
            },
            Case {
                source: r#"struct Channels(
    #[cfg(any())]
    Result<[(); {
        /// Hidden input.
        struct Input; 0
    }], [(); {
        /// Hidden output.
        struct Output; 0
    }]>,
    [(); {
        /// Active channel.
        struct Channel; 0
    }],
);"#,
                expected_texts: &["Active channel."],
            },
            Case {
                source: r#"fn select(code: u8) {
    match code {
        #[cfg(any())]
        0 if ready::<[(); {
            /// Hidden arm input.
            struct Input; 0
        }], [(); {
            /// Hidden arm output.
            struct Output; 0
        }]>() => {
            /// Hidden guarded arm.
            struct Guarded;
        },
        _ => {
            /// Active selection.
            struct Selection;
        },
    }
}"#,
                expected_texts: &["Active selection."],
            },
            Case {
                source: r#"fn discard(code: u8) {
    match code {
        #[cfg(any())]
        0 => core::mem::drop::<Result<[(); {
            /// Discarded request.
            struct Request; 0
        }], [(); {
            /// Discarded response.
            struct Response; 0
        }]>>(Ok([])),
        _ => {
            /// Active discard.
            struct Discard;
        },
    }
}"#,
                expected_texts: &["Active discard."],
            },
            Case {
                source: r#"fn adjust(code: u8) {
    match code {
        #[cfg(any())]
        0 => {
            /// Hidden value.
            struct Value; 0u8
        }.wrapping_add({
            /// Hidden increment.
            struct Increment; 1
        }),
        _ => {
            /// Active adjustment.
            struct Adjustment;
        },
    }
}"#,
                expected_texts: &["Active adjustment."],
            },
            Case {
                source: r#"fn measure(code: u8) {
    let _length = match code {
        #[cfg(any())]
        0 => (const {
            /// Hidden base length.
            struct Base; 0usize
        }) + const {
            /// Hidden extra length.
            struct Extra; 1usize
        } + const {
            /// Hidden final length.
            struct Final; 2usize
        },
        _ => {
            /// Active measurement.
            struct Measurement; 0usize
        },
    };
}"#,
                expected_texts: &["Active measurement."],
            },
            Case {
                source: r#"fn retry(code: u8) {
    match code {
        #[cfg(any())]
        0 => {
            /// Hidden retry.
            struct Retry;
        }
        _ => {
            /// Active retry.
            struct Recovery;
        }
    }
}"#,
                expected_texts: &["Active retry."],
            },
        ];
        for Case {
            source,
            expected_texts,
        } in cases
        {
            syn::parse_file(source).expect("the generic regression is valid Rust syntax");
            assert_documentation(source, expected_texts);
        }
    }

    /// A traversed block's flat statement count must not consume the next sibling's docs.
    #[test]
    fn recovery_keeps_siblings_after_a_completed_block() {
        let statements = "let _ = 0;".repeat(17);
        let source = format!(
            r#"fn inspect() {{ {{ {statements} }} /// Independent helper.
struct Independent; /// Following helper.
struct Following; }}"#
        );
        let expected_texts = ["Independent helper.", "Following helper."];
        let expected_warning_count = 0;
        let clusters = Clusters::load_from_str(&source, true, false).unwrap();
        let actual_warning_count = clusters.warnings().len();
        assert_extracted_documentation(&source, clusters, &expected_texts);
        assert_eq!(actual_warning_count, expected_warning_count);
    }

    /// A wide parameter list must not hide documented functions following its body.
    #[test]
    fn recovery_keeps_siblings_after_a_wide_function() {
        let parameter_count = 65;
        let parameters =
            Vec::from_iter((0..parameter_count).map(|index| format!("arg_{index}: u8"))).join(", ");
        let source = format!(
            r#"#[cfg(any())]
fn wide({parameters}) {{}}
/// Independent function.
fn independent() {{}}
/// Following function.
fn following() {{}}"#
        );
        let expected_texts = ["Independent function.", "Following function."];
        let expected_warning_count = 0;
        let clusters = Clusters::load_from_str(&source, true, false).unwrap();
        let actual_warning_count = clusters.warnings().len();
        assert_extracted_documentation(&source, clusters, &expected_texts);
        assert_eq!(actual_warning_count, expected_warning_count);
    }

    /// A wide tuple variant must end at its own comma rather than the next variant's docs.
    #[test]
    fn recovery_keeps_siblings_after_a_wide_variant() {
        let field_count = 130;
        let fields = vec!["u8"; field_count].join(", ");
        let source = format!(
            r#"enum Packet {{
    #[cfg(any())]
    Wide({fields}),
    /// Independent variant.
    Independent,
    /// Following variant.
    Following,
}}"#
        );
        let expected_texts = ["Independent variant.", "Following variant."];
        let expected_warning_count = 0;
        let clusters = Clusters::load_from_str(&source, true, false).unwrap();
        let actual_warning_count = clusters.warnings().len();
        assert_extracted_documentation(&source, clusters, &expected_texts);
        assert_eq!(actual_warning_count, expected_warning_count);
    }

    /// A rejected generic header recovers at the next sibling attribute, not its semicolon.
    #[test]
    fn recovery_keeps_siblings_after_a_rejected_generic_header() {
        let parameter_count = 140;
        let parameters =
            Vec::from_iter((0..parameter_count).map(|index| format!("Link{index}"))).join(", ");
        let source = format!(
            r#"#[cfg(any())]
fn wide<{parameters}>() {{}}
/// Independent generic neighbor.
fn independent() {{}}
/// Following generic neighbor.
fn following() {{}}"#
        );
        let expected_texts = [
            "Independent generic neighbor.",
            "Following generic neighbor.",
        ];
        let expected_warning_count = 1;
        let expected_kind = ExtractionWarningKind::UnsupportedAttribute;
        let clusters = Clusters::load_from_str(&source, true, false).unwrap();
        assert_eq!(clusters.warnings().len(), expected_warning_count);
        assert_eq!(clusters.warnings()[0].kind, expected_kind);
        assert_extracted_documentation(&source, clusters, &expected_texts);
    }

    /// Recovery ignores generic commas but releases the failed field's guard at its own comma.
    #[test]
    fn recovery_ignores_generic_commas_before_the_next_field() {
        let generic_depth = 40;
        let opening = "Envelope<".repeat(generic_depth);
        let closing = ">".repeat(generic_depth);
        let source = format!(
            r#"#[cfg(storage)]
struct State {{
    #[cfg(any())]
    hidden: {opening}Result<[(); {{
        /// Hidden request size.
        struct Request; 0
    }}], [(); {{
        /// Hidden response size.
        struct Response; 0
    }}]>{closing},
    visible: [(); {{
        /// Separate field.
        struct Field; 0
    }}],
}}"#
        );
        let expected_texts = ["Separate field."];
        let expected_warning_count = 1;
        let expected_witness = Some("storage");
        let clusters = Clusters::load_from_str(&source, true, false).unwrap();
        assert_eq!(clusters.warnings().len(), expected_warning_count);
        let chunks = assert_extracted_documentation(&source, clusters, &expected_texts);
        assert_eq!(chunks[0].cfg_example(), expected_witness);
    }

    /// Flat cfg width uses the symbolic variable budget rather than classifier token limits.
    #[test]
    fn flat_cfg_predicates_fit_the_default_variable_budget_on_a_small_stack() {
        let test_name =
            "cluster::tests::flat_cfg_predicates_fit_the_default_variable_budget_on_a_small_stack";
        small_stack_probe(test_name, || {
            let atom_counts = [256, 1024];
            for atom_count in atom_counts {
                let atoms = Vec::from_iter(
                    (0..atom_count).map(|index| format!("route = \"segment_{index}\"")),
                )
                .join(", ");
                let source = format!(
                    r#"#[cfg(all({atoms}))]
/// Wide transport guide.
struct TransportGuide;
/// Independent wide neighbor.
struct WideNeighbor;"#
                );
                let expected_texts = ["Wide transport guide.", "Independent wide neighbor."];
                assert_documentation(&source, &expected_texts);
                let conditional_source = format!(
                    r#"/// Conditional transport guide.
#[cfg_attr(all({atoms}), doc = "All routes.")]
struct ConditionalGuide;"#
                );
                let expected_conditional_texts = [
                    "Conditional transport guide.",
                    "Conditional transport guide. All routes.",
                ];
                assert_documentation(&conditional_source, &expected_conditional_texts);
            }
        });
    }

    /// Run abort-prone extraction on a bounded child process with a fixed small thread stack.
    fn small_stack_probe(test_name: &str, probe: fn()) {
        let child_marker = "DOC_CHUNKS_STACK_PROBE";
        if std::env::var(child_marker).ok().as_deref() == Some(test_name) {
            let stack_bytes = 2 * 1024 * 1024;
            std::thread::Builder::new()
                .stack_size(stack_bytes)
                .spawn(probe)
                .unwrap()
                .join()
                .unwrap();
            return;
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture"])
            .env(child_marker, test_name)
            .status()
            .unwrap();
        let expected_success = true;
        assert_eq!(status.success(), expected_success, "probe: {test_name}");
    }

    /// Lexing, cloning, dropping, and rejecting a deep attribute must precede recursive syn parsing.
    #[test]
    fn deep_attribute_preflight_survives_a_small_stack() {
        let test_name = "cluster::tests::deep_attribute_preflight_survives_a_small_stack";
        small_stack_probe(test_name, || {
            let predicate_depth = 4096;
            let predicate = format!(
                "{}compressed{}",
                "not(".repeat(predicate_depth),
                ")".repeat(predicate_depth)
            );
            let source = format!(
                r#"/// Unavailable compression guide.
#[cfg({predicate})]
mod unavailable {{
    /// Unavailable descendant.
    struct Descendant;
}}
/// Independent compression guide.
struct IndependentCompression;"#
            );

            let expected_texts = ["Independent compression guide."];
            let expected_warning_count = 1;
            let expected_kind = ExtractionWarningKind::UnsupportedAttribute;
            let expected_warning_source = format!("[cfg({predicate})]");
            let clusters = Clusters::load_from_str(&source, true, false).unwrap();
            assert_eq!(clusters.warnings().len(), expected_warning_count);
            let warning = &clusters.warnings()[0];
            assert_eq!(warning.kind, expected_kind);
            let actual_warning_source =
                crate::util::load_span_from(source.as_bytes(), warning.span).unwrap();
            assert_eq!(actual_warning_source, expected_warning_source);
            assert_extracted_documentation(&source, clusters, &expected_texts);
        });
    }

    /// Shallow delimiter nesting must not permit unbounded recursive expression classification.
    #[test]
    fn operator_preflight_survives_a_small_stack() {
        let test_name = "cluster::tests::operator_preflight_survives_a_small_stack";
        small_stack_probe(test_name, || {
            let operator_count = 512;
            let operators = "!".repeat(operator_count);
            let source = format!(
                r#"/// Inspection guide.
fn inspect() {{
    /// Unsafe expression documentation.
    let _ = {operators} {{
        /// Unsafe expression descendant.
        struct UnsafeHelper;
        false
    }};
    /// Independent inspection helper.
    struct IndependentInspection;
}}
/// Independent outside helper.
struct IndependentOutside;"#
            );
            let expected_texts = [
                "Inspection guide.",
                "Independent inspection helper.",
                "Independent outside helper.",
            ];
            let expected_warning_count = 1;
            let expected_kind = ExtractionWarningKind::UnsupportedAttribute;
            let expected_message = "complexity limit exceeded";
            let clusters = Clusters::load_from_str(&source, true, false).unwrap();
            assert_eq!(clusters.warnings().len(), expected_warning_count);
            let warning = &clusters.warnings()[0];
            assert_eq!(warning.kind, expected_kind);
            assert!(warning.message.contains(expected_message));
            assert_extracted_documentation(&source, clusters, &expected_texts);
        });
    }

    /// Both accepted and rejected classifier inputs remain safe on the fixed small stack.
    #[test]
    fn classifier_boundary_survives_a_small_stack() {
        let test_name = "cluster::tests::classifier_boundary_survives_a_small_stack";
        small_stack_probe(test_name, || {
            /// An operator density paired with its warning count and reachable documentation.
            struct Case {
                operator_count: usize,
                expected_warning_count: usize,
                expected_texts: &'static [&'static str],
            }
            let cases = [
                Case {
                    operator_count: 8,
                    expected_warning_count: 0,
                    expected_texts: &["Nested boundary helper.", "Independent boundary helper."],
                },
                Case {
                    operator_count: 24,
                    expected_warning_count: 0,
                    expected_texts: &["Nested boundary helper.", "Independent boundary helper."],
                },
                Case {
                    operator_count: 32,
                    expected_warning_count: 1,
                    expected_texts: &["Independent boundary helper."],
                },
            ];
            for Case {
                operator_count,
                expected_warning_count,
                expected_texts,
            } in cases
            {
                let operators = "!".repeat(operator_count);
                let source = format!(
                    r#"fn inspect() {{
    let _ = {operators} {{
        /// Nested boundary helper.
        struct BoundaryHelper;
        false
    }};
    /// Independent boundary helper.
    struct IndependentBoundary;
}}"#
                );
                let clusters = Clusters::load_from_str(&source, true, false).unwrap();
                assert_eq!(clusters.warnings().len(), expected_warning_count);
                assert_extracted_documentation(&source, clusters, expected_texts);
            }
        });
    }

    /// Literal text length does not measure parser recursion or token complexity.
    #[test]
    fn long_documentation_literals_do_not_consume_classification_complexity() {
        let prose = "Detailed transport documentation. ".repeat(4096);
        let source = format!(
            r#"#[doc = "{prose}"]
struct TransportGuide;"#
        );
        let expected_text = prose.trim();
        let expected_texts = [expected_text];
        assert_documentation(&source, &expected_texts);
    }

    /// Const and static item initializers retain expression-list sibling boundaries.
    #[test]
    fn item_initializer_constraints_do_not_leak_to_sibling_groups() {
        /// An initializer layout paired with the documentation from its independent element.
        struct Case {
            source: &'static str,
            expected_texts: &'static [&'static str],
        }
        let cases = [
            Case {
                source: r#"const TRANSPORT: ((),) = (
    #[cfg(any())] (),
    {
        /// Visible constant helper.
        struct ConstantHelper;
    },
);"#,
                expected_texts: &["Visible constant helper."],
            },
            Case {
                source: r#"static ROUTES: [(); 1] = [
    #[cfg(any())] (),
    {
        /// Visible static helper.
        struct StaticHelper;
    },
];"#,
                expected_texts: &["Visible static helper."],
            },
        ];
        for Case {
            source,
            expected_texts,
        } in cases
        {
            syn::parse_file(source).expect("the initializer regression is valid Rust syntax");
            assert_documentation(source, expected_texts);
        }
    }

    /// Guarded call arguments in an if condition do not constrain later arguments.
    #[test]
    fn if_condition_constraints_do_not_leak_to_sibling_groups() {
        let source = r#"fn ready(_: bool) -> bool { true }
fn inspect() {
    if ready(
        #[cfg(any())] false,
        {
            /// Visible condition helper.
            struct ConditionHelper;
            true
        },
    ) {}
}"#;
        let expected_texts = ["Visible condition helper."];
        syn::parse_file(source).expect("the if-condition regression is valid Rust syntax");
        assert_documentation(source, &expected_texts);
    }

    /// Guarded call arguments in a while condition do not constrain later arguments.
    #[test]
    fn while_condition_constraints_do_not_leak_to_sibling_groups() {
        let source = r#"fn poll(_: bool) -> bool { false }
fn inspect() {
    while poll(
        #[cfg(any())] false,
        {
            /// Visible polling helper.
            struct PollingHelper;
            false
        },
    ) {}
}"#;
        let expected_texts = ["Visible polling helper."];
        syn::parse_file(source).expect("the while-condition regression is valid Rust syntax");
        assert_documentation(source, &expected_texts);
    }

    /// A closure body's expression list owns its arguments independently of the closure.
    #[test]
    fn closure_body_constraints_do_not_leak_to_sibling_groups() {
        let source = r#"fn sink(_: ()) {}
fn inspect() {
    let _callback = || sink(
        #[cfg(any())] (),
        {
            /// Visible closure helper.
            struct ClosureHelper;
        },
    );
}"#;
        let expected_texts = ["Visible closure helper."];
        syn::parse_file(source).expect("the closure-body regression is valid Rust syntax");
        assert_documentation(source, &expected_texts);
    }

    /// Function parameter commas release a sibling guard before traversing the next type.
    #[test]
    fn parameter_constraints_do_not_leak_to_sibling_groups() {
        let source = r#"fn inspect(
    #[cfg(any())] hidden: (),
    visible: [(); {
        /// Visible parameter helper.
        struct ParameterHelper;
        0
    }],
) {}"#;
        let expected_texts = ["Visible parameter helper."];
        syn::parse_file(source).expect("the parameter regression is valid Rust syntax");
        assert_documentation(source, &expected_texts);
    }

    /// Struct literal field commas release a sibling guard before traversing the next value.
    #[test]
    fn literal_field_constraints_do_not_leak_to_sibling_groups() {
        let source = r#"struct Reading { visible: () }
fn inspect() {
    let _reading = Reading {
        #[cfg(any())] hidden: (),
        visible: {
            /// Visible reading helper.
            struct ReadingHelper;
        },
    };
}"#;
        let expected_texts = ["Visible reading helper."];
        syn::parse_file(source).expect("the literal-field regression is valid Rust syntax");
        assert_documentation(source, &expected_texts);
    }

    /// Tuple expression commas end element guards rather than constraining the next element.
    #[test]
    fn tuple_element_constraints_do_not_leak_to_sibling_groups() {
        let source = r#"fn inspect() {
    let _pair = (
        #[cfg(any())] (),
        {
            /// Visible tuple helper.
            struct TupleHelper;
        },
    );
}"#;
        let expected_texts = ["Visible tuple helper."];
        syn::parse_file(source).expect("the tuple-element regression is valid Rust syntax");
        assert_documentation(source, &expected_texts);
    }

    /// Array expression commas end element guards rather than constraining the next element.
    #[test]
    fn array_element_constraints_do_not_leak_to_sibling_groups() {
        let source = r#"fn inspect() {
    let _values = [
        #[cfg(any())] (),
        {
            /// Visible array helper.
            struct ArrayHelper;
        },
    ];
}"#;
        let expected_texts = ["Visible array helper."];
        syn::parse_file(source).expect("the array-element regression is valid Rust syntax");
        assert_documentation(source, &expected_texts);
    }

    /// Call argument commas end argument guards without constraining the next argument.
    #[test]
    fn call_argument_constraints_do_not_leak_to_sibling_groups() {
        let source = r#"fn sink(_: ()) {}
fn inspect() {
    sink(
        #[cfg(any())] (),
        {
            /// Visible argument helper.
            struct ArgumentHelper;
        },
    );
}"#;
        let expected_texts = ["Visible argument helper."];
        syn::parse_file(source).expect("the call-argument regression is valid Rust syntax");
        assert_documentation(source, &expected_texts);
    }

    /// List owners keep generic commas and chained const groups within the guarded sibling.
    #[test]
    fn parameter_and_expression_list_countercases_keep_their_constraints() {
        /// A guarded list layout paired with documentation from its independent sibling.
        struct Case {
            source: &'static str,
            expected_texts: &'static [&'static str],
        }
        let cases = [
            Case {
                source: r#"fn connect(
    #[cfg(any())]
    hidden: Result<[(); {
        /// Hidden request.
        struct Request; 0
    }], [(); {
        /// Hidden response.
        struct Response; 0
    }]>,
    visible: [(); {
        /// Active connection.
        struct Connection; 0
    }],
) {}"#,
                expected_texts: &["Active connection."],
            },
            Case {
                source: r#"struct Report { visible: () }
fn report() {
    let _report = Report {
        #[cfg(any())]
        hidden: core::mem::drop::<Result<[(); {
            /// Hidden report input.
            struct Input; 0
        }], [(); {
            /// Hidden report output.
            struct Output; 0
        }]>>(Ok([])),
        visible: {
            /// Active report.
            struct Summary;
        },
    };
}"#,
                expected_texts: &["Active report."],
            },
            Case {
                source: r#"fn record() {
    let _records = (
        #[cfg(any())]
        core::mem::drop::<Result<[(); {
            /// Hidden record input.
            struct Input; 0
        }], [(); {
            /// Hidden record output.
            struct Output; 0
        }]>>(Ok([])),
        {
            /// Active recording.
            struct Record;
        },
    );
}"#,
                expected_texts: &["Active recording."],
            },
            Case {
                source: r#"fn sink(_: usize) {}
fn measure() {
    sink(
        #[cfg(any())]
        ((const {
            /// Hidden initial count.
            struct Initial; 0usize
        }) + const {
            /// Hidden extra count.
            struct Extra; 1usize
        } + const {
            /// Hidden final count.
            struct Final; 2usize
        }),
        {
            /// Active count.
            struct Count; 0usize
        },
    );
}"#,
                expected_texts: &["Active count."],
            },
            Case {
                source: r#"fn allocate() {
    let _buffers = [
        #[cfg(any())]
        ((|| {
            /// Hidden allocator.
            struct Allocator; 0usize
        })() + {
            /// Hidden increment.
            struct Increment; 1usize
        }),
        {
            /// Active allocation.
            struct Allocation; 0usize
        },
    ];
}"#,
                expected_texts: &["Active allocation."],
            },
        ];
        for Case {
            source,
            expected_texts,
        } in cases
        {
            syn::parse_file(source).expect("the list countercase is valid Rust syntax");
            assert_documentation(source, expected_texts);
        }
    }

    /// A completed statement block releases only its own pending constraints.
    #[test]
    fn block_statement_constraints_do_not_leak_to_following_blocks() {
        let source = r#"fn check() {
    #[cfg(any())]
    {
        /// Invisible helper.
        struct Invisible;
    }
    {
        /// Visible helper.
        struct Visible;
    }
}"#;
        let expected_texts = ["Visible helper."];
        assert_documentation(source, &expected_texts);
    }

    /// Control-flow bodies retain their guard through else branches, then release it.
    #[test]
    fn control_flow_statement_constraints_end_after_the_complete_expression() {
        let source = r#"fn inspect() {
    #[cfg(any())]
    if ready() {
        /// Invisible ready handler.
        struct ReadyHandler;
    } else {
        /// Invisible waiting handler.
        struct WaitingHandler;
    }
    {
        /// Visible inspection helper.
        struct Inspection;
    }
    #[cfg(any())]
    loop {
        /// Invisible retry handler.
        struct RetryHandler;
        break;
    }
    {
        /// Visible completion helper.
        struct Completion;
    }
}"#;
        let expected_texts = ["Visible inspection helper.", "Visible completion helper."];
        assert_documentation(source, &expected_texts);
    }

    /// Generic commas and expression groups remain part of their constrained owner.
    #[test]
    fn generic_and_const_groups_retain_pending_constraints() {
        /// A non-boundary layout paired with its independently reachable documentation.
        struct Case {
            source: &'static str,
            expected_texts: &'static [&'static str],
        }
        let cases = [
            Case {
                source: r#"#[cfg(any())]
struct Cache<const WIDTH: usize = {
    /// Hidden width.
    struct Width; 4
}, const HEIGHT: usize = {
    /// Hidden height.
    struct Height; 8
}> {
    /// Hidden storage.
    storage: (),
}
/// Separate cache.
struct SeparateCache;"#,
                expected_texts: &["Separate cache."],
            },
            Case {
                source: r#"fn resize() {
    #[cfg(any())]
    let capacity = {
        /// Hidden base capacity.
        struct Base; 4
    } + {
        /// Hidden extra capacity.
        struct Extra; 8
    };
    {
        /// Active allocator.
        struct Allocator;
    }
}"#,
                expected_texts: &["Active allocator."],
            },
            Case {
                source: r#"fn register() {
    #[cfg(any())]
    install::<[u8; {
        /// Hidden input buffer.
        struct Input; 4
    }], [u8; {
        /// Hidden output buffer.
        struct Output; 8
    }]>();
    {
        /// Active registration.
        struct Registration;
    }
}"#,
                expected_texts: &["Active registration."],
            },
            Case {
                source: r#"fn configure() {
    #[cfg(any())]
    {
        /// Hidden builder.
        struct Builder; builder()
    }.configure({
        /// Hidden settings.
        struct Settings; settings()
    });
    {
        /// Active configuration.
        struct Configuration;
    }
}"#,
                expected_texts: &["Active configuration."],
            },
            Case {
                source: r#"enum Signal {
    #[cfg(any())]
    Hidden = signal_code::<u8, {
        /// Hidden signal code.
        struct Code; 4
    }>(),
    Active {
        /// Active signal.
        payload: (),
    },
}"#,
                expected_texts: &["Active signal."],
            },
            Case {
                source: r#"enum Buffer {
    #[cfg(any())]
    Hidden(Result<[u8; {
        /// Hidden request length.
        struct Request; 4
    }], [u8; {
        /// Hidden response length.
        struct Response; 8
    }]>),
    Active {
        /// Active buffer payload.
        payload: (),
    },
}"#,
                expected_texts: &["Active buffer payload."],
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
