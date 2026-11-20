//! Bound recursive parser work before invoking syn on untrusted token streams.

use proc_macro2::{Delimiter, TokenStream, TokenTree};

use super::attributes::inclusive_span;
use super::{ExtractionWarning, ExtractionWarningKind};

/// Maximum delimiter and traversal nesting admitted to recursive parsing on a small stack.
pub(super) const MAX_PARSER_DEPTH: usize = 32;
/// Maximum token count admitted to a single classifier or attribute parser invocation.
const MAX_PARSER_TOKENS: usize = 256;
/// Maximum combined nesting and recursive operator density admitted to a parser invocation.
const MAX_PARSER_RECURSION: usize = 32;

/// A source-located reason to skip an owner before invoking a recursive parser.
pub(super) struct ParsingLimit {
    pub(super) span: proc_macro2::Span,
    pub(super) reason: &'static str,
}

impl ParsingLimit {
    /// Reuse the existing unsupported-attribute diagnostic for safely skipped owners.
    pub(super) fn warning(&self) -> ExtractionWarning {
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
pub(super) fn parsing_limit(stream: &TokenStream) -> Option<ParsingLimit> {
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
pub(super) fn attribute_parsing_limit(stream: &TokenStream) -> Option<ParsingLimit> {
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
