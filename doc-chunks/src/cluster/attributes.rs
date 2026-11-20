//! Parse documentation attributes while retaining guarded payloads and source spans.

use proc_macro2::TokenStream;
use syn::ext::IdentExt;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Expr, Ident, Lit, LitStr, Macro, MacroDelimiter, Meta, Token};

use super::{ExtractionWarning, ExtractionWarningKind};
use crate::cfg::Predicate;
use crate::Span;

#[cfg(test)]
mod tests;

/// Parsed `doc = ...` syntax, before loading source-backed documentation fragments.
pub(super) enum DocAssignment {
    LitStr(LitStr),
    AttributedLitStr(LitStr),
    Macro(Macro),
    NonDocLiteral,
    Unsupported(Span),
}

/// Convert an exclusive token span to the inclusive source coordinates used by chunks.
pub(super) fn inclusive_span(span: proc_macro2::Span) -> Span {
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
                Lit::Str(value) if value.suffix().is_empty() => {
                    if literal.attrs.is_empty() {
                        Self::LitStr(value)
                    } else {
                        Self::AttributedLitStr(value)
                    }
                }
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
pub(super) enum DocAttribute {
    Doc(DocAssignment),
    Conditional {
        condition: SpannedPredicate,
        attributes: Vec<DocAttribute>,
    },
    Cfg(SpannedPredicate),
    Unsupported(ExtractionWarning),
    Other,
}

/// A fragment guard or owner constraint with its source span for coverage diagnostics.
#[derive(Clone)]
pub(super) struct SpannedPredicate {
    pub(super) predicate: Predicate,
    pub(super) span: Span,
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

fn parse_cfg(content: syn::parse::ParseStream<'_>) -> syn::Result<DocAttribute> {
    let condition = parse_spanned_predicate(content)?;
    content.parse::<Option<Token![,]>>()?;
    if !content.is_empty() {
        return Err(content.error("expected a single cfg predicate"));
    }
    Ok(DocAttribute::Cfg(condition))
}

fn parse_cfg_attr(content: syn::parse::ParseStream<'_>) -> syn::Result<DocAttribute> {
    let condition = parse_spanned_predicate(content)?;
    content.parse::<Token![,]>()?;
    let payload = content.parse::<TokenStream>()?;
    let attributes =
        match Punctuated::<DocAttribute, Token![,]>::parse_terminated.parse2(payload.clone()) {
            Ok(attributes) => Vec::from_iter(attributes),
            // rustc need not validate discarded attribute expressions as eagerly as
            // syn does. Preserve the parsed parent guard even if child parsing fails.
            Err(error) => vec![DocAttribute::Unsupported(ExtractionWarning {
                kind: ExtractionWarningKind::UnsupportedAttribute,
                span: inclusive_span(payload.span()),
                message: format!(
                    "Skipping documentation variants with unsupported attribute payloads: {error}"
                ),
            })],
        };
    Ok(DocAttribute::Conditional {
        condition,
        attributes,
    })
}

fn parse_configuration_attribute(name: &Ident, meta: &Meta) -> syn::Result<DocAttribute> {
    let Meta::List(list) = meta else {
        return Err(syn::Error::new_spanned(
            meta,
            "expected parenthesized configuration attribute",
        ));
    };
    if !matches!(list.delimiter, MacroDelimiter::Paren(_)) {
        return Err(syn::Error::new_spanned(
            meta,
            "expected parentheses for configuration attribute",
        ));
    }
    if name == "cfg" {
        list.parse_args_with(parse_cfg)
    } else {
        list.parse_args_with(parse_cfg_attr)
    }
}

impl syn::parse::Parse for DocAttribute {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        // Keep Expr parsing for doc values: MetaNameValue folds negative literals
        // into Expr::Lit and rejects expression attributes, changing extraction policy.
        if input
            .fork()
            .parse::<Ident>()
            .is_ok_and(|name| name.unraw() == "doc")
            && input.peek2(Token![=])
        {
            return input.parse().map(Self::Doc);
        }
        let meta = input.parse::<Meta>()?;
        let Some(name) = meta.path().get_ident().map(IdentExt::unraw) else {
            return Ok(Self::Other);
        };
        if name != "cfg" && name != "cfg_attr" {
            return Ok(Self::Other);
        }
        let span = meta.span();
        let parsed = parse_configuration_attribute(&name, &meta);
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
