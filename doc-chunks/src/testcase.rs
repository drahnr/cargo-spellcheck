use crate::{Span, TrimmedLiteral};

pub fn annotated_literals_raw(source: &str) -> impl Iterator<Item = proc_macro2::Literal> + '_ {
    let stream = syn::parse_str::<proc_macro2::TokenStream>(source).expect("Must be valid rust");
    stream
        .into_iter()
        .filter_map(|x| {
            if let proc_macro2::TokenTree::Group(group) = x {
                Some(group.stream().into_iter())
            } else {
                None
            }
        })
        .flatten()
        .filter_map(|x| {
            if let proc_macro2::TokenTree::Literal(literal) = x {
                Some(literal)
            } else {
                None
            }
        })
}

pub fn annotated_literals(source: &str) -> Vec<TrimmedLiteral> {
    annotated_literals_raw(source)
        .map(|literal| {
            let span = Span::from(literal.span());
            TrimmedLiteral::load_from(source, span)
                .expect("Literals must be convertable to trimmed literals")
        })
        .collect()
}
