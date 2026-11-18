//! Symbolic configuration predicates used to project conditional documentation.
//!
//! We deliberately do not evaluate the host's configuration: documentation for
//! other targets and disabled features needs spellchecking as well.

use syn::ext::IdentExt;
use syn::punctuated::Punctuated;
use syn::{Expr, Lit, LitBool, Meta, Token};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Atom {
    Flag(String),
    Value(String, String),
}

#[derive(Clone, Debug)]
pub(super) enum Predicate {
    Constant(bool),
    Atom(Atom),
    Not(Box<Predicate>),
    All(Vec<Predicate>),
    Any(Vec<Predicate>),
}

impl syn::parse::Parse for Predicate {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(LitBool) {
            return Ok(Self::Constant(input.parse::<LitBool>()?.value));
        }
        Self::from_meta(input.parse()?)
    }
}

impl Predicate {
    pub(super) fn always() -> Self {
        Self::Constant(true)
    }

    fn from_meta(meta: Meta) -> syn::Result<Self> {
        let name = meta
            .path()
            .get_ident()
            .map(|ident| ident.unraw().to_string());
        match (name.as_deref(), meta) {
            (Some(name), Meta::Path(_)) => Ok(Self::Atom(Atom::Flag(name.to_owned()))),
            (Some(name), Meta::NameValue(value)) => match value.value {
                Expr::Lit(syn::ExprLit {
                    lit: Lit::Str(value),
                    ..
                }) => Ok(Self::Atom(Atom::Value(name.to_owned(), value.value()))),
                other => Err(syn::Error::new_spanned(
                    other,
                    "expected a cfg string value",
                )),
            },
            (Some("all" | "any" | "not"), Meta::List(list)) => {
                let predicates = list
                    .parse_args_with(Punctuated::<Predicate, Token![,]>::parse_terminated)?
                    .into_iter()
                    .collect::<Vec<_>>();
                match name.as_deref() {
                    Some("all") => Ok(Self::All(predicates)),
                    Some("any") => Ok(Self::Any(predicates)),
                    _ if predicates.len() == 1 => {
                        Ok(Self::Not(Box::new(predicates.into_iter().next().unwrap())))
                    }
                    _ => Err(syn::Error::new_spanned(
                        list.path,
                        "not() requires exactly one predicate",
                    )),
                }
            }
            (_, other) => Err(syn::Error::new_spanned(other, "unsupported cfg predicate")),
        }
    }

    pub(super) fn collect_atoms(&self, atoms: &mut Vec<Atom>) {
        match self {
            Self::Constant(_) => {}
            Self::Atom(atom) => {
                if !atoms.contains(atom) {
                    atoms.push(atom.clone());
                }
            }
            Self::Not(predicate) => predicate.collect_atoms(atoms),
            Self::All(predicates) | Self::Any(predicates) => {
                for predicate in predicates {
                    predicate.collect_atoms(atoms);
                }
            }
        }
    }

    pub(super) fn matches(&self, atoms: &[Atom], assignment: usize) -> bool {
        match self {
            Self::Constant(value) => *value,
            Self::Atom(atom) => {
                let index = atoms
                    .iter()
                    .position(|candidate| candidate == atom)
                    .unwrap();
                assignment & (1 << index) != 0
            }
            Self::Not(predicate) => !predicate.matches(atoms, assignment),
            Self::All(predicates) => predicates
                .iter()
                .all(|predicate| predicate.matches(atoms, assignment)),
            Self::Any(predicates) => predicates
                .iter()
                .any(|predicate| predicate.matches(atoms, assignment)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> syn::Result<Predicate> {
        syn::parse_str(source)
    }

    #[test]
    fn boolean_literals_do_not_create_atoms() {
        for (source, expected) in [
            ("true", true),
            ("false", false),
            ("not(false)", true),
            ("all(true, not(false))", true),
            ("any(false, not(true))", false),
        ] {
            let predicate = parse(source).unwrap();
            let mut atoms = Vec::new();
            predicate.collect_atoms(&mut atoms);
            assert!(atoms.is_empty(), "{source}");
            assert_eq!(predicate.matches(&atoms, 0), expected, "{source}");
        }
    }

    #[test]
    fn nested_predicates_keep_repeated_atoms_correlated() {
        let predicate = parse(r#"all(any(feature = "a", unix,), not(feature = "a"),)"#).unwrap();
        let mut atoms = Vec::new();
        predicate.collect_atoms(&mut atoms);
        assert_eq!(
            atoms,
            vec![
                Atom::Value("feature".into(), "a".into()),
                Atom::Flag("unix".into())
            ]
        );
        assert!(!predicate.matches(&atoms, 0));
        assert!(!predicate.matches(&atoms, 1));
        assert!(predicate.matches(&atoms, 2));
        assert!(!predicate.matches(&atoms, 3));
    }

    #[test]
    fn raw_predicate_names_share_atoms_with_plain_names() {
        let predicate = parse(r#"all(r#unix, unix, r#feature = "a", feature = "a")"#).unwrap();
        let mut atoms = Vec::new();
        predicate.collect_atoms(&mut atoms);
        assert_eq!(
            atoms,
            vec![
                Atom::Flag("unix".into()),
                Atom::Value("feature".into(), "a".into())
            ]
        );
        assert!(predicate.matches(&atoms, 3));
    }

    #[test]
    fn empty_combinators_and_invalid_predicates() {
        assert!(parse("all()").unwrap().matches(&[], 0));
        assert!(!parse("any()").unwrap().matches(&[], 0));
        for source in [
            "not()",
            "not(unix, windows)",
            "all(42)",
            "feature = 42",
            "feature = concat!(\"a\")",
            "unknown(unix)",
            "some::flag",
        ] {
            assert!(parse(source).is_err(), "unexpectedly accepted {source}");
        }
    }
}
