//! Representation of multiple documents.
//!
//! So to speak documentation of project as whole.

use super::*;
use crate::LiteralSet;

use std::convert::TryInto;

use indexmap::IndexMap;
use log::trace;
use proc_macro2::{Spacing, TokenTree};

pub use proc_macro2::LineColumn;
use std::path::{Path, PathBuf};

/// Collection of all the documentation entries across the project
#[derive(Debug, Clone)]
pub struct Documentation {
    /// Mapping of a path to documentation literals
    // @todo add an intermediate enum to be able to handle markdown files as part of a document too
    index: IndexMap<PathBuf, Vec<LiteralSet>>,
}

impl Documentation {
    pub fn new() -> Self {
        Self {
            index: IndexMap::with_capacity(64),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&PathBuf, &Vec<LiteralSet>)> {
        self.index.iter()
    }

    pub fn into_iter(self) -> impl Iterator<Item = (PathBuf, Vec<LiteralSet>)> {
        self.index.into_iter()
    }

    pub fn join(&mut self, other: Documentation) -> &mut Self {
        other
            .into_iter()
            .for_each(|(path, mut literals): (_, Vec<LiteralSet>)| {
                self.index
                    .entry(path)
                    .and_modify(|acc: &mut Vec<LiteralSet>| {
                        acc.append(&mut literals);
                    })
                    .or_insert_with(|| literals);
            });
        self
    }

    pub fn combine(mut docs: Vec<Documentation>) -> Documentation {
        if let Some(first) = docs.pop() {
            docs.into_iter().fold(first, |mut first, other| {
                first.join(other);
                first
            })
        } else {
            Documentation::new()
        }
    }

    /// Append a literal to the given path
    ///
    /// Only works if the file is processed line by line, otherwise
    /// requires a adjacency list.
    pub fn append_literal(&mut self, path: &Path, literal: proc_macro2::Literal) {
        let literal = TrimmedLiteral::from(literal);
        match self.index.entry(path.to_owned()) {
            indexmap::map::Entry::Occupied(occupied) => {
                let v = occupied.into_mut();
                let cls = v.last_mut().unwrap();
                if let Err(literal) = cls.add_adjacent(literal) {
                    trace!(target: "documentation",
                        "appending, but failed to append: {:?} to set {:?}",
                        &literal,
                        &cls
                    );
                    v.push(LiteralSet::from(literal))
                } else {
                    trace!("successfully appended to existing: {:?} to set", &cls);
                }
            }
            indexmap::map::Entry::Vacant(vacant) => {
                trace!(target: "documentation",
                    "nothing for {} file yet, create new literal set",
                    path.display()
                );
                vacant.insert(vec![LiteralSet::from(literal)]);
            }
        }
    }

    /// Helper function to parse a path stream and associated the found literals to `path`
    fn parse_token_tree<P: AsRef<Path>>(&mut self, path: P, stream: proc_macro2::TokenStream) {
        let path: &Path = path.as_ref();

        let mut iter = stream.into_iter();
        while let Some(tree) = iter.next() {
            match tree {
                TokenTree::Ident(ident) => {
                    // if we find an identifier
                    // which is doc
                    if ident != "doc" {
                        continue;
                    }

                    // this assures the sequence is as anticipated
                    let op = iter.next();
                    if op.is_none() {
                        continue;
                    }
                    let op = op.unwrap();
                    if let TokenTree::Punct(punct) = op {
                        if punct.as_char() != '=' {
                            continue;
                        }
                        if punct.spacing() != Spacing::Alone {
                            continue;
                        }
                    } else {
                        continue;
                    }

                    let comment = iter.next();
                    if comment.is_none() {
                        continue;
                    }
                    let comment = comment.unwrap();
                    if let TokenTree::Literal(literal) = comment {
                        trace!(target: "documentation",
                            "Found doc literal at {:?}: {:?}",
                            <Span as TryInto<Range>>::try_into(Span::from(literal.span())),
                            literal
                        );
                        self.append_literal(path, literal);
                    } else {
                        continue;
                    }
                }
                TokenTree::Group(group) => {
                    self.parse_token_tree(path, group.stream());
                }
                _ => {}
            };
        }
    }
}

impl<P> From<(P, proc_macro2::TokenStream)> for Documentation
where
    P: AsRef<Path>,
{
    fn from((path, stream): (P, proc_macro2::TokenStream)) -> Self {
        let mut documentation = Documentation::new();
        documentation.parse_token_tree(path, stream);
        documentation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::From;

    #[test]
    fn parse_and_construct() {
        let _ = env_logger::from_env(
            env_logger::Env::new().filter_or("CARGO_SPELLCHECK", "cargo_spellcheck=trace"),
        )
        .is_test(true)
        .try_init();

        const TEST_SOURCE: &str = r#"/// **A** _very_ good test.
        struct Vikings;
        "#;

        const TEST_RAW: &str = r#" **A** _very_ good test."#;
        const TEST_PLAIN: &str = r#"A very good test."#;

        let test_path = PathBuf::from("/tmp/dummy");

        let stream = syn::parse_str(TEST_SOURCE).expect("Must be valid rust");
        let docs = Documentation::from((test_path.as_path(), stream));
        assert_eq!(docs.index.len(), 1);
        let v = docs.index.get(&test_path).expect("Must contain dummy path");
        assert_eq!(dbg!(v).len(), 1);
        assert_eq!(v[0].to_string(), TEST_RAW.to_owned());
        let plain = v[0].erase_markdown();

        println!("{:?}", &plain);

        assert_eq!(TEST_PLAIN, plain.as_str());

        //>0123456789ABCDEF
        //> **A** _very_ good test.
        let expected_raw_range = 8..12;

        // markdown does not care about leading spaces:
        //>0123456789
        //>A very good test.
        let expected_plain_range = 2..6;

        // @todo the range here is correct
        assert_eq!("very", &dbg!(plain.as_str())[expected_plain_range.clone()]);

        let z: Vec<(&TrimmedLiteral, Span)> = plain.linear_range_to_spans(expected_plain_range);
        // FIXME the expected result would be
        let (literal, span) = z.first().unwrap().clone();
        let _range: Range = span.try_into().expect("Span must be single line");
        println!(
            "full: {}",
            TrimmedLiteralRangePrint::from((literal, expected_raw_range.clone()))
        );
        assert_eq!(
            dbg!(&z),
            dbg!(&v[0].linear_range_to_spans(expected_raw_range))
        );
    }

    macro_rules! end2end_file {
        ($name: ident, $path: literal, $n: expr) => {
            #[test]
            fn $name() {
                let _ = env_logger::from_env(
                    env_logger::Env::new().filter_or("CARGO_SPELLCHECK", "cargo_spellcheck=trace"),
                )
                .is_test(true)
                .try_init();

                const TEST: &str = include_str!($path);
                let test_path = PathBuf::from($path);
                let stream = syn::parse_str(TEST).expect("Must be valid rust");
                let docs = Documentation::from((test_path.as_path(), stream));
                assert_eq!(docs.index.len(), 1);
                let v = docs.index.get(&test_path).expect("Must contain dummy path");
                assert_eq!(dbg!(v).len(), 1);
                let plain = v[0].erase_markdown();
                log::info!("{:?}", &plain);

                let config = crate::config::Config::load().unwrap_or_else(|_e| {
                    warn!("Using default configuration!");
                    Config::default()
                });
                let suggestion_set = crate::checker::check(&docs, &config)
                    .expect("Must not fail to extract suggestions");
                let (_, suggestions) = suggestion_set
                    .into_iter()
                    .next()
                    .expect("Must contain exactly one item");
                assert_eq!(dbg!(&suggestions).len(), $n);
            }
        };
    }

    end2end_file!(one, "./tests/justone.rs", 1);
    end2end_file!(two, "./tests/justtwo.rs", 2);

    // use crate::literalset::tests::{annotated_literals,gen_literal_set};

    #[cfg(feature = "hunspell")]
    #[test]
    fn end2end_chunk() {
        let _ = env_logger::from_env(
            env_logger::Env::new().filter_or("CARGO_SPELLCHECK", "cargo_spellcheck=trace"),
        )
        .is_test(true)
        .try_init();

        let _config = crate::config::Config::load().unwrap_or_else(|_e| {
            warn!("Using default configuration!");
            Config::default()
        });

        const SOURCE: &'static str = r#"/// A headline.
///
/// Erronbeous **bold** __uetchkp__
struct X"#;

        const RAW: &'static str = r#" A headline.

 Erronbeous **bold** __uetchkp__"#;

        const PLAIN: &'static str = r#"A headline.

Erronbeous bold uetchkp"#;

        let config = crate::config::Config::default();
        let stream =
            syn::parse_str::<proc_macro2::TokenStream>(SOURCE).expect("Must parse just fine");
        let path = PathBuf::from("/tmp/virtual");
        let docs = crate::documentation::Documentation::from((&path, stream));

        let suggestion_set = dbg!(crate::checker::check(&docs, &config)).expect("Must not error");
        let (path2, literal_set) = docs.iter().next().expect("Must contain exactly one");
        assert_eq!(&path, path2);

        let literal_set_no1 = literal_set
            .first()
            .expect("Must cotain at least one literalset");
        assert_eq!(literal_set.len(), 1);
        assert_eq!(RAW, literal_set_no1.to_string().as_str());
        assert_eq!(PLAIN, literal_set_no1.erase_markdown().as_str());

        let mut it = suggestion_set.iter();
        let (_, suggestions) = dbg!(it.next()).expect("Must contain at least one file entry");

        let mut it = suggestions.into_iter();
        let mut expected = |word: &'static str| {
            let suggestion = it.next().expect("Must contain one mis-spelled word");
            let range: Range = suggestion.span.try_into().expect("Must be a single line");
            let s = dbg!(suggestion.literal.as_ref().as_untrimmed_str());
            let x =
                crate::TrimmedLiteralRangePrint::from((suggestion.literal.as_ref(), range.clone()));
            println!("Foxxy funkster: {:?}", x);
            assert_eq!(word, &s[range]);
            println!("Found word >> {} <<", word);
        };

        expected("Erronbeous");
        expected("uetchkp");
    }
}
