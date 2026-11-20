//! Parse documentation attributes while retaining guarded payloads and source spans.

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use syn::ext::IdentExt;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{parenthesized, Expr, Ident, Lit, LitStr, Macro, Token};

use super::{ExtractionWarning, ExtractionWarningKind};
use crate::cfg::Predicate;
use crate::Span;

/// Parsed `doc = ...` syntax, before loading source-backed documentation fragments.
pub(super) enum DocAssignment {
    LitStr(LitStr),
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

fn parse_configuration_attribute(
    name: &str,
    group: Option<Group>,
    span: proc_macro2::Span,
) -> syn::Result<DocAttribute> {
    let group = group
        .ok_or_else(|| syn::Error::new(span, "expected parenthesized configuration attribute"))?;
    if group.delimiter() != Delimiter::Parenthesis {
        return Err(syn::Error::new(
            span,
            "expected parentheses for configuration attribute",
        ));
    }
    if name == "cfg" {
        parse_cfg.parse2(group.stream())
    } else {
        parse_cfg_attr.parse2(group.stream())
    }
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
        let Some(name @ ("cfg" | "cfg_attr")) = name.as_deref() else {
            return Ok(Self::Other);
        };
        let parsed = parse_configuration_attribute(name, group, span);
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

#[cfg(test)]
mod tests {
    use super::DocAssignment;

    #[test]
    fn doc_assignment_parse() {
        let _ = syn::parse_str::<DocAssignment>(r########"doc=foo!(bar!(xxx))"########).unwrap();
        let _ = syn::parse_str::<DocAssignment>(r########"doc="s""########).unwrap();
        let _ = syn::parse_str::<DocAssignment>(r########"doc=r#"s"#"########).unwrap();
        let _ = syn::parse_str::<DocAssignment>(r########"doc=r##"s"##"########).unwrap();
        let _ = syn::parse_str::<DocAssignment>(r########"doc=r###"s"###"########).unwrap();
        let _ = syn::parse_str::<DocAssignment>(r########"doc=r####"s"####"########).unwrap();
    }
}
