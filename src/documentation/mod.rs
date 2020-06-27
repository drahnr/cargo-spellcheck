//! Representation of multiple documents.
//!
//! So to speak documentation of project as whole.

use super::*;

use indexmap::IndexMap;
use log::trace;
use std::convert::{TryInto, TryFrom};
use std::path::PathBuf;
pub use proc_macro2::LineColumn;
use proc_macro2::{Spacing, TokenTree};

pub type Range = core::ops::Range<usize>;

mod chunk;
mod cluster;
mod literal;
mod literalset;
mod markdown;

pub use chunk::*;
pub use cluster::*;
pub use literal::*;
pub use literalset::*;
pub use markdown::*;
/// Collection of all the documentation entries across the project
#[derive(Debug, Clone)]
pub struct Documentation {
    /// Mapping of a path to documentation literals
    index: IndexMap<ContentOrigin, Vec<CheckableChunk>>,
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

    pub fn iter(&self) -> impl Iterator<Item = (&ContentOrigin, &Vec<CheckableChunk>)> {
        self.index.iter()
    }

    pub fn into_iter(self) -> impl Iterator<Item = (ContentOrigin, Vec<CheckableChunk>)> {
        self.index.into_iter()
    }

    pub fn join(&mut self, other: Documentation) -> &mut Self {
        other
            .into_iter()
            .for_each(|(source, chunks): (_, Vec<CheckableChunk>)| {
                let _ = self.add(source, chunks);
            });
        self
    }

    pub fn extend<I, J>(&mut self, docs: I)
    where
        I: IntoIterator<Item = Documentation, IntoIter = J>,
        J: Iterator<Item = Documentation>,
    {
        docs.into_iter().for_each(|other| {
            self.join(other);
        });
    }

    pub fn add(&mut self, source: ContentOrigin, mut chunks: Vec<CheckableChunk>) {
        self.index
            .entry(source)
            .and_modify(|acc: &mut Vec<CheckableChunk>| {
                acc.append(&mut chunks);
            })
            .or_insert_with(|| chunks);
        // Ok(()) @todo make this failable
    }
}

/// only a shortcut to avoid duplicate code
impl From<(ContentOrigin, proc_macro2::TokenStream)> for Documentation {
    fn from((source, stream): (ContentOrigin, proc_macro2::TokenStream)) -> Self {
        let cluster = Clusters::try_from(stream).expect("Must succeed to create cluster from stream");
        let chunks = Vec::<CheckableChunk>::from(cluster);
        let mut docs = Documentation::new();
        docs.add(source, chunks);
        docs
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
        let origin = ContentOrigin::RustSourceFile(test_path.clone());
        let stream =
            syn::parse_str::<proc_macro2::TokenStream>(TEST_SOURCE).expect("Must be valid rust");
        let docs = Documentation::from((origin.clone(), stream));
        assert_eq!(docs.index.len(), 1);
        let chunks = docs.index.get(&origin).expect("Must contain dummy path");
        assert_eq!(dbg!(chunks).len(), 1);

        // @todo
        assert_eq!(chunks[0].as_str(), TEST_RAW.to_owned());
        let plain = chunks[0].erase_markdown();
        println!("{:?}", &plain);

        assert_eq!(TEST_PLAIN, plain.as_str());

        //>0123456789ABCDEF
        //> **A** _very_ good test.
        let expected_raw_range = 8..12;

        // markdown does not care about leading spaces:
        //>0123456789
        //>A very good test.
        let expected_plain_range = 2..6;

        assert_eq!("very", &dbg!(plain.as_str())[expected_plain_range.clone()]);

        let z: IndexMap<Range, Span> = plain.find_spans(expected_plain_range);
        // FIXME the expected result would be
        let (range, span) = z.iter().next().unwrap().clone();

        // @todo must be implemented for `CheckableChunk`
        // println!(
        //     "full: {}",
        //     TrimmedLiteralDisplay::from((literal, expected_raw_range.clone()))
        // );
        assert_eq!(
            dbg!(&z),
            dbg!(&chunks[0].find_spans(expected_raw_range))
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

                const TEST: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path));
                let test_path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path));
                let origin = ContentOrigin::RustSourceFile(test_path.clone());
                let stream =
                    syn::parse_str::<proc_macro2::TokenStream>(TEST).expect("Must be valid rust");
                let docs = Documentation::from((origin.clone(), stream));
                assert_eq!(docs.index.len(), 1);
                let v = docs.index.get(&origin).expect("Must contain dummy path");
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

    end2end_file!(one, "demo/src/nested/justone.rs", 1);
    end2end_file!(two, "demo/src/nested/justtwo.rs", 2);

    // use crate::literalset::tests::{annotated_literals,gen_literal_set};

    #[cfg(feature = "hunspell")]
    #[test]
    fn end2end_chunk() {
        let _ = env_logger::from_env(
            env_logger::Env::new().filter_or("CARGO_SPELLCHECK", "cargo_spellcheck=trace"),
        )
        .is_test(true)
        .try_init();

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
        let origin = ContentOrigin::RustSourceFile(PathBuf::from("/tmp/virtual"));
        let docs = crate::documentation::Documentation::from((origin.clone(), stream));

        let suggestion_set = crate::checker::check(&docs, &config).expect("Must not error");
        let (origin2, chunks) = docs.iter().next().expect("Must contain exactly one");
        assert_eq!(&origin, origin2);

        let chunk = chunks
            .iter()
            .next()
            .expect("Must cotain at least one literalset");
        // @todo
        // assert_eq!(chunk.len(), 1);
        assert_eq!(RAW, chunk.as_str());
        assert_eq!(PLAIN, chunk.erase_markdown().as_str());

        let mut it = suggestion_set.iter();
        let (_, suggestions) = it.next().expect("Must contain at least one file entry");

        let mut it = suggestions.into_iter();
        let mut expected = |word: &'static str| {
            let suggestion = it.next().expect("Must contain one mis-spelled word");
            let range: Range = suggestion.span.try_into().expect("Must be a single line");
            let s = dbg!(suggestion.chunk.as_str());
            println!(
                "Foxxy funkster: {:?}",
                suggestion.chunk.display(range.clone())
            );
            assert_eq!(word, &s[range]);
            println!("Found word >> {} <<", word);
        };

        expected("Erronbeous");
        expected("uetchkp");
    }
}
