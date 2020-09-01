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

use anyhow::{anyhow, Result};
use indexmap::IndexMap;
use log::trace;
pub use proc_macro2::LineColumn;
use proc_macro2::{Spacing, TokenTree};
use std::convert::{TryFrom, TryInto};
use std::path::PathBuf;

/// Range based on `usize`, simplification.
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
    /// Create a new and empty doc.
    pub fn new() -> Self {
        Self {
            index: IndexMap::with_capacity(64),
        }
    }

    /// Check if the document contains any checkable items.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Borrowing iterator across content origins and associated sets of chunks.
    #[inline(always)]
    pub fn iter(&self) -> impl Iterator<Item = (&ContentOrigin, &Vec<CheckableChunk>)> {
        self.index.iter()
    }

    /// Consuming iterator across content origins and associated sets of chunks.
    #[inline(always)]
    pub fn into_iter(self) -> impl Iterator<Item = (ContentOrigin, Vec<CheckableChunk>)> {
        self.index.into_iter()
    }

    /// Join `self` with another doc to form a new one.
    pub fn join(&mut self, other: Documentation) -> &mut Self {
        other
            .into_iter()
            .for_each(|(origin, chunks): (_, Vec<CheckableChunk>)| {
                let _ = self.add_inner(origin, chunks);
            });
        self
    }

    /// Extend `self` by joining in other `Documentation`s.
    pub fn extend<I, J>(&mut self, docs: I)
    where
        I: IntoIterator<Item = Documentation, IntoIter = J>,
        J: Iterator<Item = Documentation>,
    {
        docs.into_iter().for_each(|other| {
            self.join(other);
        });
    }

    /// Adds a set of `CheckableChunk`s to the documentation to be checked.
    fn add_inner(&mut self, origin: ContentOrigin, mut chunks: Vec<CheckableChunk>) {
        self.index
            .entry(origin)
            .and_modify(|acc: &mut Vec<CheckableChunk>| {
                acc.append(&mut chunks);
            })
            .or_insert_with(|| chunks);
        // Ok(()) TODO make this failable
    }

    /// Adds a rust content str to the documentation.
    pub fn add_rust(&mut self, origin: ContentOrigin, content: &str) -> Result<()> {
        let cluster = Clusters::try_from(content)?;

        let chunks = Vec::<CheckableChunk>::from(cluster);
        self.add_inner(origin, chunks);
        Ok(())
    }

    /// Adds a common mark content str to the documentation.
    pub fn add_commonmark(&mut self, origin: ContentOrigin, content: &str) -> Result<()> {
        // extract the full content span and range
        let start = LineColumn { line: 1, column: 0 };
        let end = content
            .lines()
            .enumerate()
            .last()
            .map(|(idx, linecontent)| (idx + 1, linecontent))
            .map(|(linenumber, linecontent)| LineColumn {
                line: linenumber,
                column: linecontent.chars().count().saturating_sub(1),
            })
            .ok_or_else(|| anyhow!("Common mark / markdown file does not contain a single line"))?;

        let span = Span { start, end };
        let source_mapping = indexmap::indexmap! {
            0..content.chars().count() => span
        };
        self.add_inner(
            origin,
            vec![CheckableChunk::from_str(content, source_mapping)],
        );
        Ok(())
    }

    /// get funky
    #[inline(always)]
    pub fn get(&self, origin: &ContentOrigin) -> Option<&[CheckableChunk]> {
        self.index.get(origin).map(AsRef::as_ref)
    }

    /// get funky
    #[inline(always)]
    pub fn entry_count(&self) -> usize {
        self.index.len()
    }
}

/// only a shortcut to avoid duplicate code
impl From<(ContentOrigin, &str)> for Documentation {
    fn from((origin, content): (ContentOrigin, &str)) -> Self {
        let mut docs = Documentation::new();

        match &origin {
            ContentOrigin::RustDocTest(_path, span) => {
                if let Ok(excerpt) =
                    crate::util::load_span_from(&mut content.as_bytes(), span.clone())
                {
                    docs.add_rust(origin.clone(), excerpt.as_str())
                } else {
                    // TODO
                    Ok(())
                }
            }
            ContentOrigin::RustSourceFile(_path) => docs.add_rust(origin, content),
            ContentOrigin::CommonMarkFile(_path) => docs.add_commonmark(origin, content),
            #[cfg(test)]
            ContentOrigin::TestEntityRust => docs.add_rust(origin, content),
            #[cfg(test)]
            ContentOrigin::TestEntityCommonMark => docs.add_commonmark(origin, content),
        }
        .unwrap_or_else(|e| warn!("BUG! << failed to load yada >> {}", e));
        docs
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::checker::Checker;
    use crate::util::load_span_from;
    use crate::{chyrp_up, fluff_up};

    use std::convert::From;

    #[test]
    fn parse_and_construct() {
        let _ = env_logger::builder()
            .is_test(true)
            .filter(None, log::LevelFilter::Trace)
            .try_init();

        const TEST_SOURCE: &str = r#"/// **A** _very_ good test.
        struct Vikings;
        "#;

        const TEST_RAW: &str = r#" **A** _very_ good test."#;
        const TEST_PLAIN: &str = r#"A very good test."#;

        let origin = ContentOrigin::TestEntityRust;
        let docs = Documentation::from((origin.clone(), TEST_SOURCE));
        assert_eq!(docs.index.len(), 1);
        let chunks = docs.index.get(&origin).expect("Must contain dummy path");
        assert_eq!(dbg!(chunks).len(), 1);

        // TODO
        let chunk = &chunks[0];
        assert_eq!(chunk.as_str(), TEST_RAW.to_owned());
        let plain = chunk.erase_cmark();
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

        assert_eq!(
            "very".to_owned(),
            sub_chars(plain.as_str(), expected_plain_range.clone())
        );

        let z: IndexMap<Range, Span> = plain.find_spans(expected_plain_range);
        // FIXME the expected result would be
        let (_range, _span) = z.iter().next().unwrap().clone();

        let chunk = &chunks[0];
        log::trace!("full: {}", chunk.display(expected_raw_range.clone()));
        assert_eq!(z, chunk.find_spans(expected_raw_range));
    }

    // use crate::literalset::tests::{annotated_literals,gen_literal_set};
    use crate::checker::dummy::DummyChecker;
    use crate::documentation::Documentation;

    /// Declare an end to end test case, ranging from input content
    /// down to the number expected issues given a checker type.
    #[macro_export]
    macro_rules! end2end {
        ($test:expr, $n:expr) => {{
            end2end!($test, ContentOrigin::TestEntityRust, $n, DummyChecker);
        }};

        ($test:expr, $origin:expr, $n:expr, $checker:ty) => {{
            let _ = env_logger::builder()
                .is_test(true)
                .filter(None, log::LevelFilter::Trace)
                .try_init();

            let origin: ContentOrigin = $origin;
            let docs = Documentation::from((origin.clone(), $test));
            assert_eq!(docs.index.len(), 1);
            let chunks = docs.index.get(&origin).expect("Must contain dummy path");
            assert_eq!(dbg!(chunks).len(), 1);
            let chunk = &chunks[0];
            let _plain = chunk.erase_cmark();

            let cfg = Default::default();
            let suggestion_set = <$checker>::check(&docs, &cfg)
                .expect("Must not fail to extract suggestions");
            let (_, suggestions) = suggestion_set
                .iter()
                .next()
                .expect("Must contain exactly one item");
            assert_eq!(suggestions.len(), $n);
        }};
    }

    /// Declare an end to end test case based on an existing rust file.
    macro_rules! end2end_file_rust {
        ($path: literal, $n: expr) => {{
            let path2 = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path));
            let origin = ContentOrigin::RustSourceFile(path2);
            end2end!(
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path)),
                origin,
                $n,
                DummyChecker
            );
        }};
    }

    /// Declare an end to end test case based on an existing common mark file.
    macro_rules! end2end_file_cmark {
        ($path: literal, $n: expr) => {{
            let path2 = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path));
            let origin = ContentOrigin::CommonMarkFile(path2);
            end2end!(
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path)),
                origin,
                $n,
                DummyChecker
            );
        }};
    }

    mod e2e {
        use super::*;

        #[test]
        fn tripleslash_one_line() {
            end2end!(fluff_up!(["Uni"]), 1);
        }

        #[test]
        fn tripleslash_two_lines() {
            end2end!(fluff_up!(["Alphy", "Beto"]), 2);
        }

        #[test]
        fn macro_doc_one_line() {
            end2end!(chyrp_up!(["Uni"]), 1);
        }

        #[test]
        fn macro_doc_two_lines() {
            end2end!(chyrp_up!(["Alphy", "Beto"]), 2);
        }

        #[test]
        fn file_justone() {
            end2end_file_rust!("demo/src/nested/justone.rs", 1);
        }

        #[test]
        fn file_justtwo() {
            end2end_file_rust!("demo/src/nested/justtwo.rs", 2);
        }

        // use crate::literalset::tests::{annotated_literals,gen_literal_set};
        use crate::checker::dummy::DummyChecker;
        use crate::documentation::Documentation;

        /// Verifies the extracted spans and ranges are covering the expected words.
        macro_rules! bananasplit {
            ($origin:expr; $source:ident -> $raw:ident -> $plain:ident expect [ $( $x:literal ),* $(,)? ]) => {
                let _ = env_logger::builder()
                    .is_test(true)
                    .filter(None, log::LevelFilter::Trace)
                    .try_init();

                let _source = $source;

                let origin: ContentOrigin = $origin;

                let docs = Documentation::from((origin.clone(), $source));

                let suggestion_set =
                    dbg!(DummyChecker::check(&docs, &())).expect("Dummy checker never fails. qed");

                let (origin2, chunks) = docs
                    .iter()
                    .next()
                    .expect("Introduced exactly one source. qed");
                assert_eq!(&origin, origin2);

                let chunk = chunks
                    .iter()
                    .next()
                    .expect("Commonmark files always contain a chunk. qed");

                assert_eq!(chunks.len(), 1);
                assert_eq!(RAW, chunk.as_str());

                let plain = chunk.erase_cmark();
                assert_eq!($plain, plain.as_str());

                let mut it = suggestion_set.iter();
                let (_, suggestions) = it.next().expect("Dummy checker produces one error per tokenized word. qed");

                let mut it = suggestions.into_iter();

                let mut expected = |word: &'static str| {
                    log::info!("Working on expected token >{}<", word);
                    let suggestion = dbg!(it.next()).expect("Number of words is by test design equal to the number of expects. qed");
                    let _s = dbg!(suggestion.chunk.as_str());

                    // range for chunk
                    let range: Range = suggestion
                        .span
                        .to_content_range(&suggestion.chunk)
                        .expect("Must work to derive content range from chunk and span");

                    log::info!(
                        "Current assumed word based on `Range`: {}",
                        suggestion.chunk.display(range.clone())
                    );

                    log::info!("Checking word boundaries of >{}< against the chunk/range", word);
                    assert_eq!(word, crate::util::sub_chars(chunk.as_str(), range));

                    log::info!("Checking word boundaries of >{}< against the source/span", word);
                    let alternative = load_span_from($source.as_bytes(), suggestion.span.clone())
                        .expect("Span loading must succeed");

                    assert_eq!(word, alternative);
                };

                $(expected($x);
                )*
            };
        }

        #[test]
        fn word_extraction_rust() {
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

            bananasplit!(ContentOrigin::TestEntityRust;
                SOURCE -> RAW -> PLAIN
                expect
                [
                    "A",
                    "headline"
                ]
            );
        }

        #[test]
        fn word_extraction_commonmark() {
            // raw source
            const SOURCE: &'static str = r#"# cmark test

<pre>🌡</pre>

A relly boring test.

## Engineering

```rust
I am so code!
```

---

**Breakage** ` ```rust` anticipated?

The end.🐢"#;

            // extracted content as present as provided by `chunk.as_str()`
            const RAW: &'static str = SOURCE;

            // markdown erased residue
            const PLAIN: &'static str = r#"cmark test

A relly boring test.

Engineering


Breakage  anticipated?

The end.🐢"#;

            bananasplit!(
                ContentOrigin::TestEntityCommonMark;
                SOURCE -> RAW -> PLAIN
                expect
                [
                    "cmark",
                    "test",
                    "A",
                    "relly",
                    "boring",
                    "test",
                    "Engineering",
                    "Breakage",
                    "anticipated",
                    "The",
                    "end",
                    "🐢",
                ]
            );
        }

        #[test]
        fn word_extraction_emoji() {
            // TODO FIXME remove the 🍁, and observe early termination

            // raw source
            const SOURCE: &'static str = r#"A
x🌡  in 🍁

---

Ef gh"#;

            // extracted content as present as provided by `chunk.as_str()`
            const RAW: &'static str = SOURCE;

            // markdown erased residue
            const PLAIN: &'static str = r#"A
x🌡  in 🍁


Ef gh"#;

            bananasplit!(
                ContentOrigin::TestEntityCommonMark;
                SOURCE -> RAW -> PLAIN
                expect
                [
                    "A",
                    "x🌡",
                    "in",
                    "🍁",
                    "Ef",
                    "gh",
                ]
            );
        }

        #[test]
        fn word_extraction_issue_104_thermostat() {
            // TODO FIXME remove the 🍁, and observe early termination

            // raw source
            const SOURCE: &'static str = r#"
Ref1

🌡🍁

Ref2

<pre>🌡</pre>

Ref3

`🌡`

Ref4
"#;

            // extracted content as present as provided by `chunk.as_str()`
            const RAW: &'static str = SOURCE;

            // markdown erased residue
            const PLAIN: &'static str = r#"Ref1

🌡🍁

Ref2

Ref3



Ref4"#;

            bananasplit!(
                ContentOrigin::TestEntityCommonMark;
                SOURCE -> RAW -> PLAIN
                expect
                [
                    "Ref1",
                    "🌡🍁",
                    "Ref2",
                    "Ref3",
                    "Ref4",
                ]
            );
        }
    }
}
