//! Representation of multiple documents.
//!
//! So to speak documentation of project as whole.
//!
//! A `literal` is a token provided by `proc_macro2`, which is then
//! converted by means of `TrimmedLiteral` using `Cluster`ing
//! into a `CheckableChunk` (mostly named just `chunk`).
//!
//! `CheckableChunk`s can consist of multiple fragments, where
//! each fragment can span multiple lines, yet each fragment
//! is covering a consecutive `Span` in the origin content.
//! Each fragment also has a direct mapping to the `CheckableChunk` internal
//! string representation.

use super::*;

use indexmap::IndexMap;
use log::trace;
pub use proc_macro2::LineColumn;
use proc_macro2::{Spacing, TokenTree};
use std::convert::{TryFrom, TryInto};
use std::path::PathBuf;

pub type Range = core::ops::Range<usize>;

mod chunk;
mod cluster;
mod literal;
pub(crate) mod literalset;
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
impl From<(ContentOrigin, &str)> for Documentation {
    fn from((source, content): (ContentOrigin, &str)) -> Self {
        let cluster =
            Clusters::try_from(content).expect("Must succeed to create cluster from content");
        let chunks = Vec::<CheckableChunk>::from(cluster);
        let mut docs = Documentation::new();
        docs.add(source, chunks);
        docs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::bandaid::tests::load_span_from;
    use crate::checker::Checker;
    use crate::fluff_up;

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
        let docs = Documentation::from((origin.clone(), TEST_SOURCE));
        assert_eq!(docs.index.len(), 1);
        let chunks = docs.index.get(&origin).expect("Must contain dummy path");
        assert_eq!(dbg!(chunks).len(), 1);

        // @todo
        let chunk = &chunks[0];
        assert_eq!(chunk.as_str(), TEST_RAW.to_owned());
        let plain = chunk.erase_markdown();
        println!("{:?}", &plain);

        assert_eq!(TEST_PLAIN, plain.as_str());

        // ```text
        // " **A** _very_ good test."
        //  0123456789ABCDEF01234567
        // ```
        let expected_raw_range = 8..12;

        // markdown does not care about leading spaces:
        //
        // ```text
        // "A very good test."
        //  0123456789ABCDEF0
        // ```
        let expected_plain_range = 2..6;

        assert_eq!("very", &dbg!(plain.as_str())[expected_plain_range.clone()]);

        let z: IndexMap<Range, Span> = plain.find_spans(expected_plain_range);
        // FIXME the expected result would be
        let (_range, _span) = z.iter().next().unwrap().clone();

        let chunk = &chunks[0];
        log::trace!("full: {}", chunk.display(expected_raw_range.clone()));
        assert_eq!(z, chunk.find_spans(expected_raw_range));
    }

    macro_rules! end2end {
        ($test:expr, $n:expr) => {
            end2end!(
                $test,
                $n,
                ContentOrigin::RustSourceFile(PathBuf::from("/tmp/dummy"))
            )
        };

        ($test:expr, $n:expr, $origin:expr) => {{
            let _ = env_logger::from_env(
                env_logger::Env::new().filter_or("CARGO_SPELLCHECK", "cargo_spellcheck=trace"),
            )
            .is_test(true)
            .try_init();

            let origin = $origin;
            let docs = Documentation::from((origin.clone(), $test));
            assert_eq!(docs.index.len(), 1);
            let chunks = docs.index.get(&origin).expect("Must contain dummy path");
            assert_eq!(dbg!(chunks).len(), 1);
            let chunk = &chunks[0];
            let _plain = chunk.erase_markdown();

            let suggestion_set = crate::checker::dummy::DummyChecker::check(&docs, &())
                .expect("Must not fail to extract suggestions");
            let (_, suggestions) = suggestion_set
                .iter()
                .next()
                .expect("Must contain exactly one item");
            assert_eq!(suggestions.len(), $n);
            suggestion_set
        }};
    }

    macro_rules! end2end_file {
        ($path: literal, $n: expr) => {{
            let path2 = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path));
            let origin = ContentOrigin::RustSourceFile(path2);
            end2end!(
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path)),
                $n,
                origin
            )
        }};
    }

    #[test]
    fn one_line() {
        end2end!(fluff_up!(["Uni"]), 1);
    }

    #[test]
    fn two_lines() {
        end2end!(fluff_up!(["Alphy", "Beto"]), 2);
    }

    #[test]
    fn one() {
        end2end_file!("demo/src/nested/justone.rs", 1);
    }

    #[test]
    fn two() {
        end2end_file!("demo/src/nested/justtwo.rs", 2);
    }

    // use crate::literalset::tests::{annotated_literals,gen_literal_set};
    use crate::checker::dummy::DummyChecker;
    use crate::documentation::Documentation;

    #[cfg(feature = "hunspell")]
    #[test]
    fn end2end_chunk() {
        let _ = env_logger::from_env(
            env_logger::Env::new().filter_or("CARGO_SPELLCHECK", "cargo_spellcheck=trace"),
        )
        .is_test(true)
        .try_init();

        // raw source
        const SOURCE: &'static str = r#"/// A headline.
///
/// Erronbeous **bold** __uetchkp__
struct X"#;

        // extracted content as present as provided by `chunk.as_str()`
        const RAW: &'static str = r#" A headline.

 Erronbeous **bold** __uetchkp__"#;

        // markdown erased residue
        const PLAIN: &'static str = r#"A headline.

Erronbeous bold uetchkp"#;

        let origin = ContentOrigin::RustSourceFile(PathBuf::from("/tmp/virtual"));
        let docs = Documentation::from((origin.clone(), SOURCE));

        // @todo contains utter garbage, should be individual tokens, but is multiple literal
        let suggestion_set = dbg!(DummyChecker::check(&docs, &())).expect("Must not error");
        let (origin2, chunks) = docs.iter().next().expect("Must contain exactly one origin");
        assert_eq!(&origin, origin2);

        let chunk = chunks
            .iter()
            .next()
            .expect("Must contain exactly one chunk");

        assert_eq!(chunks.len(), 1);
        assert_eq!(RAW, chunk.as_str());

        let plain = chunk.erase_markdown();
        assert_eq!(PLAIN, plain.as_str());

        let mut it = suggestion_set.iter();
        let (_, suggestions) = it.next().expect("Must contain at least one file entry");

        let mut it = suggestions.into_iter();
        let mut expected = |word: &'static str| {
            let suggestion = it.next().expect("Must contain another mis-spelled word");
            let _s = dbg!(suggestion.chunk.as_str());

            // range for chunk
            let range: Range = suggestion
                .span
                .to_content_range(&suggestion.chunk)
                .expect("Must work to derive content range from chunk and span");

            log::info!(
                "Foxxy funkster: {}",
                suggestion.chunk.display(range.clone())
            );

            let _alternative = load_span_from(SOURCE.as_bytes(), suggestion.span.clone())
                .expect("Span loading must succeed");

            assert_eq!(word, &chunk.as_str()[range]);
            log::info!("Found word >> {} <<", word);
        };

        expected("A");
        // expected(" A headline.\n///\n/// Erronbeous ");
        expected("headline");
    }
}
