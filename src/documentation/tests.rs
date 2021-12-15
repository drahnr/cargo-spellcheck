use super::literalset::tests::gen_literal_set;
use super::*;
use crate::checker::Checker;
use crate::util::{load_span_from, sub_char_range, sub_chars};
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
    let docs = Documentation::load_from_str(origin.clone(), TEST_SOURCE, false);
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

use crate::documentation::Documentation;

/// Declare an end-to-end test case, ranging from input content down to the
/// number expected issues given a checker type.
#[macro_export]
/// End-to-end tests for different Checkers
macro_rules! end2end {
    ($test:expr, $n:expr) => {{
        end2end!($test, ContentOrigin::TestEntityRust, $n, DummyChecker);
    }};

    ($test:expr, $origin:expr, $n:expr, $checker:ty) => {{
        let cfg = dbg!(Default::default());
        end2end!($test, $origin, $n, $checker, cfg);
    }};

    ($test:expr, $origin:expr, $n:expr, $checker:ty, $cfg:expr) => {{
        let _ = env_logger::builder()
            .is_test(true)
            .filter_level(log::LevelFilter::Trace)
            .try_init();

        let origin: ContentOrigin = $origin;
        let docs = Documentation::load_from_str(origin.clone(), $test, false);
        assert_eq!(docs.index.len(), 1);
        let chunks = docs.index.get(&origin).expect("Must contain dummy path");
        assert_eq!(dbg!(chunks).len(), 1);
        let chunk = &chunks[0];
        let _plain = chunk.erase_cmark();
        let cfg = $cfg;
        dbg!(std::any::type_name::<$checker>());
        let checker = <$checker>::new(&cfg).expect("Checker construction works");
        let suggestions = checker
            .check(&origin, &chunks[..])
            .expect("Must not fail to extract suggestions");
        assert_eq!(suggestions.len(), $n);
    }};
}

/// Declare an end-to-end test case based on an existing rust file.
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

/// Declare an end-to-end test case based on an existing common mark file.
#[allow(unused_macros)]
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

    use crate::checker::HunspellChecker;

    #[test]
    fn issue_226() {
        use crate::config::*;
        use fancy_regex::Regex;

        let transform_regex = [r#"\\\[()?:[1-9][0-9]*\\\]"#]
            .iter()
            .map(|&x| WrappedRegex(Regex::new(x).unwrap()))
            .collect::<Vec<_>>();

        let cfg = crate::config::HunspellConfig {
            // FIXME splitchars
            quirks: crate::config::Quirks {
                transform_regex,
                ..Default::default()
            },
            ..Default::default()
        };

        end2end!(
            r####"
/// X is [\[1790\]]
///
/// [\[1790\]]: https://ahoi.io
struct X;
            "####,
            ContentOrigin::TestEntityRust,
            0,
            HunspellChecker,
            cfg
        );
    }

    #[test]
    fn issue_227() {
        // The test
        end2end!(
            r####"
/// ```
/// use std::path::PathBuf as A;
#[doc = "// Hello"]
/// use std::path::PathBuf as B;
/// ```
struct X;
            "####,
            ContentOrigin::TestEntityRust,
            0,
            DummyChecker,
            Default::default()
        );
    }

    #[test]
    fn issue_234() {
        // The test
        end2end!(
            r####"
/// ```
/// use Z;
#[doc = foo!(xyz)]
/// struct X;
/// ```
struct X;
"####,
            ContentOrigin::TestEntityRust,
            0,
            DummyChecker,
            Default::default()
        );
    }

    #[test]
    fn file_justone() {
        end2end_file_rust!("demo/src/nested/justone.rs", 2);
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

            let docs = Documentation::load_from_str(origin.clone(), $source, false);
            let (origin2, chunks) = docs.into_iter().next().expect("Contains a document");
            let suggestions =
                dbg!(DummyChecker.check(&origin, &chunks[..])).expect("Dummy checker never fails. qed");

            assert_eq!(origin, origin2);

            let chunk = chunks
                .iter()
                .next()
                .expect("Commonmark files always contains a chunk. qed");

            assert_eq!(chunks.len(), 1);
            assert_eq!(RAW, chunk.as_str());

            let plain = dbg!(chunk.erase_cmark());
            assert_eq!(dbg!($plain), plain.as_str());

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
                assert_eq!(word, sub_chars(chunk.as_str(), range));

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
        const SOURCE: &str = r#"/// A headline.
///
/// Erronbeous **bold** __uetchkp__
struct X"#;

        // extracted content as present as provided by `chunk.as_str()`
        const RAW: &str = r#" A headline.

 Erronbeous **bold** __uetchkp__"#;

        // markdown erased residue
        const PLAIN: &str = r#"A headline.

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
    fn word_extraction_commonmark_small() {
        // raw source
        const SOURCE: &str = r###"## cmark test 1

🐢 are _so_ *cute*!
"###;

        // extracted content as present as provided by `chunk.as_str()`
        const RAW: &str = SOURCE;

        // markdown erased residue
        const PLAIN: &str = r##"cmark test 1

🐢 are so cute!"##;

        bananasplit!(
            ContentOrigin::TestEntityCommonMark;
            SOURCE -> RAW -> PLAIN
            expect
            [
                "cmark",
                "test",
                "1",
                "🐢",
                "are",
                "so",
                "cute",
                "!",
            ]
        );
    }

    #[test]
    fn word_extraction_commonmark_large() {
        // raw source
        const SOURCE: &str = r#"# cmark test

<pre>🌡</pre>

A relly boring test.

## Engineering

```rust
let code = be;
```

---

**Breakage** on `rust` anticipated?

The end.🐢"#;

        // extracted content as present as provided by `chunk.as_str()`
        const RAW: &str = SOURCE;

        // markdown erased residue
        const PLAIN: &str = r#"cmark test

A relly boring test.

Engineering


Breakage on rust anticipated?

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
                ".",
                "Engineering",
                "Breakage",
                "on",
                // "rust", code is not included in the list
                "anticipated",
                "?",
                "The",
                "end",
                ".",
                "🐢",
            ]
        );
    }

    #[test]
    fn word_extraction_emoji() {
        // TODO FIXME remove the 🍁, and observe early termination

        // raw source
        const SOURCE: &str = r#"A
x🌡  in 🍁

---

Ef gh"#;

        // extracted content as present as provided by `chunk.as_str()`
        const RAW: &str = SOURCE;

        // markdown erased residue
        const PLAIN: &str = r#"A
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
        const SOURCE: &str = r#"
Ref1

🌡🍁

Ref2

<pre>🌡</pre>

Ref3

`🌡`

Ref4
"#;

        // extracted content as present as provided by `chunk.as_str()`
        const RAW: &str = SOURCE;

        // markdown erased residue
        const PLAIN: &str = r#"Ref1

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

#[test]
fn find_spans_emoji() {
    const TEST: &str = r##"ab **🐡** xy"##;

    let chunk = CheckableChunk::from_str(
        TEST,
        indexmap::indexmap! { 0..11 => Span {
            start: LineColumn {
                line: 1usize,
                column: 4usize,
            },
            end: LineColumn {
                line: 1usize,
                column: 14usize,
            },
        }},
        CommentVariant::CommonMark,
    );

    assert_eq!(chunk.find_spans(0..2).len(), 1);
    assert_eq!(chunk.find_spans(5..6).len(), 1);
    assert_eq!(chunk.find_spans(9..11).len(), 1);
    assert_eq!(chunk.find_spans(9..20).len(), 1);
}

#[test]
fn find_spans_simple() {
    let _ = env_logger::builder()
        .is_test(true)
        .filter(None, log::LevelFilter::Trace)
        .try_init();

    // generate  `///<space>...`
    const SOURCE: &str = fluff_up!(["xyz"]);
    let set = gen_literal_set(SOURCE);
    let chunk = dbg!(CheckableChunk::from_literalset(set));

    // range in `chunk.as_str()`
    // " xyz"
    const CHUNK_RANGE: Range = 1..4;

    // "/// xyz"
    //  0123456
    const EXPECTED_SPAN: Span = Span {
        start: LineColumn { line: 1, column: 4 },
        end: LineColumn { line: 1, column: 6 },
    };

    let range2span = chunk.find_spans(CHUNK_RANGE.clone());
    // test deals only with a single line, so we know it only is a single entry
    assert_eq!(range2span.len(), 1);

    // assure the range is correct given the chunk
    assert_eq!("xyz", &chunk.as_str()[CHUNK_RANGE.clone()]);

    let (range, span) = dbg!(range2span.iter().next().unwrap());
    assert!(CHUNK_RANGE.contains(&(range.start)));
    assert!(CHUNK_RANGE.contains(&(range.end - 1)));
    assert_eq!(
        load_span_from(SOURCE.as_bytes(), dbg!(*span)).expect("Span extraction must work"),
        "xyz".to_owned()
    );
    assert_eq!(span, &EXPECTED_SPAN);
}

#[test]
fn find_spans_multiline() {
    let _ = env_logger::builder()
        .is_test(true)
        .filter(None, log::LevelFilter::Trace)
        .try_init();

    const SOURCE: &str = fluff_up!(["xyz", "second", "third", "Converts a span to a range, where `self` is converted to a range reltive to the",
         "passed span `scope`."] @ "       "
    );
    let set = gen_literal_set(SOURCE);
    let chunk = dbg!(CheckableChunk::from_literalset(set));
    const SPACES: usize = 7;
    const TRIPLE_SLASH_SPACE: usize = 4;
    const CHUNK_RANGES: &[Range] = &[1..4, (4 + 1 + 1 + 6 + 1 + 1)..(4 + 1 + 1 + 6 + 1 + 1 + 5)];
    const EXPECTED_SPANS: &[Span] = &[
        Span {
            start: LineColumn {
                line: 1,
                column: SPACES + TRIPLE_SLASH_SPACE + 0,
            },
            end: LineColumn {
                line: 1,
                column: SPACES + TRIPLE_SLASH_SPACE + 2,
            },
        },
        Span {
            start: LineColumn {
                line: 3,
                column: SPACES + TRIPLE_SLASH_SPACE + 0,
            },
            end: LineColumn {
                line: 3,
                column: SPACES + TRIPLE_SLASH_SPACE + 4,
            },
        },
        Span {
            start: LineColumn {
                line: 4,
                column: SPACES + TRIPLE_SLASH_SPACE + 0,
            },
            end: LineColumn {
                line: 4,
                column: SPACES + TRIPLE_SLASH_SPACE + 78,
            },
        },
        Span {
            start: LineColumn {
                line: 5,
                column: SPACES + TRIPLE_SLASH_SPACE + 0,
            },
            end: LineColumn {
                line: 5,
                column: SPACES + TRIPLE_SLASH_SPACE + 19,
            },
        },
    ];
    const EXPECTED_STR: &[&'static str] = &[
        "xyz",
        "third",
        "Converts a span to a range, where `self` is converted to a range reltive to the",
        "passed span `scope`.",
    ];

    for (query_range, expected_span, expected_str) in itertools::cons_tuples(
        CHUNK_RANGES
            .iter()
            .zip(EXPECTED_SPANS.iter())
            .zip(EXPECTED_STR.iter()),
    ) {
        let range2span = chunk.find_spans(query_range.clone());
        // test deals only with a single line, so we know it only is a single entry
        assert_eq!(range2span.len(), 1);
        let (range, span) = dbg!(range2span.iter().next().unwrap());
        assert!(query_range.contains(&(range.start)));
        assert!(query_range.contains(&(range.end - 1)));
        assert_eq!(
            load_span_from(SOURCE.as_bytes(), *span).expect("Span extraction must work"),
            expected_str.to_owned()
        );
        assert_eq!(span, expected_span);
    }
}

#[test]
fn find_spans_chyrp() {
    let _ = env_logger::builder()
        .is_test(true)
        .filter(None, log::LevelFilter::Trace)
        .try_init();

    const SOURCE: &str = chyrp_up!(["Amsel", "Wacholderdrossel", "Buchfink"]);
    let set = gen_literal_set(SOURCE);
    let chunk = dbg!(CheckableChunk::from_literalset(set));

    const CHUNK_RANGES: &[Range] = &[0..(5 + 1 + 16 + 1 + 8)];
    const EXPECTED_SPANS: &[Span] = &[Span {
        start: LineColumn {
            line: 1,
            column: 0 + 9,
        }, // prefix is #[doc=r#"
        end: LineColumn { line: 3, column: 7 }, // suffix is pointeless
    }];

    assert_eq!(
        dbg!(&EXPECTED_SPANS[0]
            .to_content_range(&chunk)
            .expect("Must be ok to extract span from chunk")),
        dbg!(&CHUNK_RANGES[0])
    );

    const EXPECTED_STR: &[&'static str] = &[r#"Amsel
Wacholderdrossel
Buchfink"#];

    assert_eq!(EXPECTED_STR[0], chunk.as_str());

    for (query_range, expected_span, expected_str) in itertools::cons_tuples(
        CHUNK_RANGES
            .iter()
            .zip(EXPECTED_SPANS.iter())
            .zip(EXPECTED_STR.iter()),
    ) {
        let range2span = chunk.find_spans(query_range.clone());
        // test deals only with a single line, so we know it only is a single entry
        assert_eq!(range2span.len(), 1);
        let (range, span) = dbg!(range2span.iter().next().unwrap());
        assert!(query_range.contains(&(range.start)));
        assert!(query_range.contains(&(range.end - 1)));
        assert_eq!(
            load_span_from(SOURCE.as_bytes(), *span).expect("Span extraction must work"),
            expected_str.to_owned()
        );
        assert_eq!(span, expected_span);
    }
}

#[test]
fn find_spans_inclusive() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    const SOURCE: &str = fluff_up!(["Some random words"]);
    let set = gen_literal_set(SOURCE);
    let chunk = dbg!(CheckableChunk::from_literalset(set));
    // a range inside the span
    const CHUNK_RANGE: Range = 4..15;

    const EXPECTED_SPAN: Span = Span {
        start: LineColumn { line: 1, column: 3 },
        end: LineColumn {
            line: 1,
            column: 20,
        },
    };

    let mut range2span = chunk.find_covered_spans(CHUNK_RANGE.clone());

    // assure the range is correct given the chunk
    assert_eq!("e random wo", &chunk.as_str()[CHUNK_RANGE.clone()]);

    let span = dbg!(range2span.next().unwrap());
    assert_eq!(
        load_span_from(SOURCE.as_bytes(), dbg!(*span)).expect("Span extraction must work"),
        " Some random words".to_owned()
    );
    assert_eq!(span, &EXPECTED_SPAN);
    // test deals only with a single line, so we know it only is a single entry
    assert_eq!(range2span.count(), 0);
}

#[test]
fn find_spans_and_coverage_integrity() -> Result<()> {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    const SOURCE: &str = fluff_up!(
        [
            "ü×ä×ß",
            "DEF_HIJ",
            "🐔🌗🐢"
        ] @ "       "
    );
    let set = dbg!(gen_literal_set(SOURCE));
    let chunk = dbg!(CheckableChunk::from_literalset(set));
    let s = chunk.as_str();
    assert_eq!(s, " ü×ä×ß\n DEF_HIJ\n 🐔🌗🐢");

    const SPACES: usize = 7;
    const TRIPLE_SLASH_SPACE: usize = 4;

    const CHUNK_RANGE_START: usize = 5 + 1 + 1; // space + content + newline
    const CHUNK_RANGE_END: usize = CHUNK_RANGE_START + 8;
    const CHUNK_RANGE: Range = CHUNK_RANGE_START..CHUNK_RANGE_END;

    const EXPECTED_SPANS: &[Span] = &[Span {
        start: LineColumn {
            line: 2,
            column: SPACES + TRIPLE_SLASH_SPACE - 1, // we want to include the whitespace here
        },
        end: LineColumn {
            line: 2,
            column: SPACES + TRIPLE_SLASH_SPACE - 1 + 7, // inclusive
        },
    }];
    dbg!(sub_char_range(s, CHUNK_RANGE.clone()));
    let coverage = chunk.find_covered_spans(CHUNK_RANGE.clone());
    let mapping = chunk.find_spans(CHUNK_RANGE.clone());

    for (coverage_span, (find_range, find_span), expected) in
        itertools::cons_tuples(coverage.zip(mapping.iter()).zip(EXPECTED_SPANS))
    {
        let cs = load_span_from(SOURCE.as_bytes(), coverage_span.clone())?;
        let fs = load_span_from(SOURCE.as_bytes(), find_span.clone())?;
        let fr = sub_char_range(chunk.as_str(), find_range.clone());
        let x = load_span_from(SOURCE.as_bytes(), expected.clone())?;
        log::trace!("[find]chunk[range]: {:?}", fr);
        log::trace!("[find]excerpt(true): {:?}", fs);
        log::trace!("[cove]excerpt(true): {:?}", cs);
        log::trace!("expected: {:?}", x);
        assert_eq!(coverage_span, expected);
        assert_eq!(find_span, expected);
        assert_eq!(fr, x);
        assert_eq!(fs, x);
        assert_eq!(cs, x);
    }
    Ok(())
}

#[test]
fn find_coverage_multiline() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    const SOURCE: &str = fluff_up!(
        [
            "xyz",
            "second",
            "third",
            "Converts a span to a range, where `self` is converted to a range reltive to the",
            "passed span `scope`."
        ] @ "       "
    );
    let set = dbg!(gen_literal_set(SOURCE));
    let chunk = dbg!(CheckableChunk::from_literalset(set));
    const SPACES: usize = 7;
    const TRIPLE_SLASH_SPACE: usize = 3;
    const CHUNK_RANGE: Range = 7..22;
    const EXPECTED_SPANS: &[Span] = &[
        Span {
            start: LineColumn {
                line: 2,
                column: SPACES + TRIPLE_SLASH_SPACE + 0,
            },
            end: LineColumn {
                line: 2,
                column: SPACES + TRIPLE_SLASH_SPACE + 6,
            },
        },
        Span {
            start: LineColumn {
                line: 3,
                column: SPACES + TRIPLE_SLASH_SPACE + 0,
            },
            end: LineColumn {
                line: 3,
                column: SPACES + TRIPLE_SLASH_SPACE + 5,
            },
        },
        Span {
            start: LineColumn {
                line: 4,
                column: SPACES + TRIPLE_SLASH_SPACE + 0,
            },
            end: LineColumn {
                line: 4,
                column: SPACES + TRIPLE_SLASH_SPACE + 79,
            },
        },
    ];

    let coverage = chunk.find_covered_spans(CHUNK_RANGE);

    for (span, expected) in coverage.zip(EXPECTED_SPANS) {
        assert_eq!(span, expected);
    }
}

#[test]
fn find_line_lengths_tripple_slash() {
    const SOURCE: &str = fluff_up!(
        [
            "xyz",
            "second",
            "third",
            "Converts a span to a range, where `self` is converted to a range reltive to the",
            "passed span `scope`."
        ]
        @ "       "
    );

    let _ = env_logger::builder()
        .is_test(true)
        .filter_level(log::LevelFilter::Trace)
        .try_init();

    let set = gen_literal_set(SOURCE);
    let chunk = dbg!(CheckableChunk::from_literalset(set));

    let lens = chunk
        .extract_line_lengths()
        .expect("Chunk in unit test must have lines.");

    // the demo creates a single `struct X` entity with `n` comments before that.
    let expected_line_count = SOURCE.lines().count() - 1;
    assert_eq!(lens.len(), expected_line_count);

    // XXX WRONG
    for (len, line) in lens.iter().zip(SOURCE.lines()) {
        assert_eq!(len, &line.len());
    }
}

#[test]
#[ignore = "prefix and suffix are not part of the accounted `extract_lines_lengths"]
fn find_line_length_docmacro() {
    const SOURCE: &str = chyrp_up!(
        [
            "xyz",
            "second",
            "third",
            "Converts a span to a range, where `self` is converted to a range relative to the",
            "passed span `scope`."
        ]
        @ "       "
    );

    let _ = env_logger::builder()
        .is_test(true)
        .filter_level(log::LevelFilter::Trace)
        .try_init();

    println!("{}", SOURCE);
    let set = gen_literal_set(SOURCE);
    let chunk = dbg!(CheckableChunk::from_literalset(set));

    let lens = chunk
        .extract_line_lengths()
        .expect("Chunk in unit test must have lines.");

    // the demo creates a single `struct X` entity with `n` comments before that.
    let expected_line_count = SOURCE.lines().count() - 1;
    assert_eq!(lens.len(), expected_line_count);

    // XXX WRONG
    for (len, line) in lens.iter().zip(SOURCE.lines()) {
        dbg!(len, &line);
        assert_eq!(*len, line.chars().count());
    }
}

#[test]
fn drill_span() {
    const TEST: &str = r##"ab **🐡** xy"##;
    let chunk = CheckableChunk::from_str(
        TEST,
        indexmap::indexmap! { 0..11 => Span {
            start: LineColumn {
                line: 1usize,
                column: 4usize,
            },
            end: LineColumn {
                line: 1usize,
                column: 14usize,
            },
        }},
        CommentVariant::CommonMark,
    );

    let plain = chunk.erase_cmark();
    assert_eq!(plain.find_spans(0..2).len(), 1);
    assert_eq!(plain.find_spans(3..4).len(), 1);
    assert_eq!(plain.find_spans(5..7).len(), 1);
    assert_eq!(plain.find_spans(5..12).len(), 1);
    assert_eq!(plain.find_spans(9..20).len(), 0);
}

#[test]
fn reduction_complex() {
    const MARKDOWN: &str = r##"# Title number 1

## Title number 2

```rust
let x = 777;
let y = 111;
let z = x/y;
assert_eq!(z,7);
```

### Title [number 3][ff]

Some **extra** _formatting_ if __anticipated__ or _*not*_ or
maybe not at all.


Extra ~pagaph~ _paragraph_.

---

And a line, or a **rule**.


[ff]: https://docs.rs
"##;

    const PLAIN: &str = r##"Title number 1

Title number 2

Title number 3

Some extra formatting if anticipated or not or
maybe not at all.

Extra ~pagaph~ paragraph.


And a line, or a rule."##;
    let (reduced, mapping) = PlainOverlay::extract_plain_with_mapping(MARKDOWN);

    assert_eq!(dbg!(&reduced).as_str(), PLAIN);
    assert_eq!(dbg!(&mapping).len(), 20);
    for (reduced_range, cmark_range) in mapping.iter() {
        assert_eq!(
            reduced[reduced_range.clone()],
            MARKDOWN[cmark_range.range()]
        );
    }
}

#[test]
fn reduction_leading_space() {
    const MARKDOWN: &str = r#"  Some __underlined__ **bold** text."#;
    const PLAIN: &str = r#"Some underlined bold text."#;

    let (reduced, mapping) = PlainOverlay::extract_plain_with_mapping(MARKDOWN);

    assert_eq!(dbg!(&reduced).as_str(), PLAIN);
    assert_eq!(dbg!(&mapping).len(), 5);
    for (reduced_range, cmark_range) in mapping.iter() {
        assert_eq!(
            reduced[reduced_range.clone()].to_owned(),
            MARKDOWN[cmark_range.range()].to_owned()
        );
    }
}

#[test]
fn range_test() {
    let mut x = IndexMap::<Range, Range>::new();
    x.insert(0..2, 1..3);
    x.insert(3..4, 7..8);
    x.insert(5..12, 11..18);

    let lookmeup = 6..8;

    // TODO keep in sync with copy pasta source, extract a func for this
    let plain_range = lookmeup;
    let v: Vec<_> = x
        .iter()
        .filter(|(plain, _md)| plain.start <= plain_range.end && plain_range.start <= plain.end)
        .fold(Vec::with_capacity(64), |mut acc, (plain, md)| {
            // calculate the linear shift
            let offset = dbg!(md.start - plain.start);
            assert_eq!(md.end - plain.end, offset);
            let extracted = Range {
                start: plain_range.start + offset,
                end: core::cmp::min(md.end, plain_range.end + offset),
            };
            acc.push(extracted);
            acc
        });
    assert_eq!(v.first(), Some(&(12..14)));
}

fn cmark_reduction_test(input: &'static str, expected: &'static str, expected_mapping_len: usize) {
    let (plain, mapping) = PlainOverlay::extract_plain_with_mapping(input);
    assert_eq!(dbg!(&plain).as_str(), expected);
    assert_eq!(dbg!(&mapping).len(), expected_mapping_len);
    for (reduced_range, markdown_range) in mapping.into_iter() {
        match markdown_range {
            SourceRange::Direct(cmark_range) => assert_eq!(
                dbg!(sub_chars(&plain, reduced_range.clone())),
                dbg!(sub_chars(&input, cmark_range))
            ),
            SourceRange::Alias(_cmark_range, _alias) => {}
        }
    }
}

#[test]
fn reduce_w_emoji() {
    cmark_reduction_test(
        r#"
Abcd

---

e🌡🍁

---

fgh"#,
        r#"Abcd


e🌡🍁


fgh"#,
        3,
    );
}

#[test]
fn reduce_w_code_block() {
    cmark_reduction_test(
        r#"
Abcd

```rust
/// Yoda is no yak!
let mut foo = unimplemented!("not yet");
```

fgh"#,
        r#"Abcd

fgh"#,
        2,
    );
}

#[test]
fn reduce_w_inline_code() {
    cmark_reduction_test(
        r#"
I like vars named `Yak<Turbo>` but not `Foo<Bar>`.
"#,
        r#"I like vars named YakTurbo but not FooBar."#,
        5,
    );
}

#[test]
fn reduce_w_link_footnote() {
    cmark_reduction_test(
        r#"footnote [^linktxt]. Which one?

[linktxt]: ../../reference/index.html"#,
        r#"footnote linktxt. Which one?"#,
        3,
    );
}

#[test]
fn reduce_w_link_inline() {
    cmark_reduction_test(
        r#" prefix [I'm an inline-style link](https://duckduckgo.com) postfix"#,
        r#"prefix I'm an inline-style link postfix"#,
        3,
    );
}
#[test]
fn reduce_w_link_auto() {
    cmark_reduction_test(
        r#" prefix <http://foo.bar/baz> postfix"#,
        r#"prefix  postfix"#,
        2,
    );
    cmark_reduction_test(r#" <http://foo.bar/baz>"#, r#""#, 0);
}

#[test]
fn reduce_w_link_email() {
    cmark_reduction_test(
        r#" prefix <loe@example.com> postfix"#,
        r#"prefix  postfix"#,
        2,
    );
}

#[test]
fn reduce_w_link_reference() {
    cmark_reduction_test(
        r#"[classy reference link][the reference str]"#,
        r#"classy reference link"#,
        1,
    );
}

#[test]
fn reduce_w_link_collapsed_ref() {
    cmark_reduction_test(
        r#"[collapsed reference link][]"#,
        r#"collapsed reference link"#,
        1,
    );
}

#[test]
fn reduce_w_link_shortcut_ref() {
    cmark_reduction_test(
        r#"[shortcut reference link]"#,
        r#"shortcut reference link"#,
        1,
    );
}
// Nested links as well as nested code blocks are
// impossible according to the common mark spec.

#[test]
fn reduce_w_list_nested() {
    cmark_reduction_test(
        r#"
* [x] a
* [ ] b
  * [ ] c
  * [x] d
"#,
        r#"
a
b
c
d"#,
        4,
    );
}

#[test]
fn reduce_w_table_ignore() {
    // TODO FIXME it would be better to transform this into
    // one line per cell and test each cell.
    // TODO very most likely will cause issues with grammar checks
    // so eventually this will have to become checker specific code
    // or handle a list of mute tags to simply ignore.
    cmark_reduction_test(
        r#"
00

|a|b|c
|-|-|-
|p|q|r

ff
"#,
        r#"00


ff"#,
        2,
    );
}

pub(crate) fn annotated_literals_raw<'a>(
    source: &'a str,
) -> impl Iterator<Item = proc_macro2::Literal> + 'a {
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

pub(crate) fn annotated_literals(source: &str) -> Vec<TrimmedLiteral> {
    annotated_literals_raw(source)
        .map(|literal| {
            let span = Span::from(literal.span());
            TrimmedLiteral::load_from(source, span)
                .expect("Literals must be convertable to trimmed literals")
        })
        .collect()
}

const PREFIX_RAW_LEN: usize = 3;
const SUFFIX_RAW_LEN: usize = 2;
const GAENSEFUESSCHEN: usize = 1;

#[derive(Clone, Debug)]
struct Triplet {
    /// source content
    source: &'static str,
    /// expected doc comment content without modifications
    #[allow(dead_code)]
    extracted: &'static str,
    /// expected doc comment content after applying trimming rules
    trimmed: &'static str,
    /// expected span as extracted by proc_macro2
    #[allow(dead_code)]
    extracted_span: Span,
    /// trimmed span, so it is aligned with the proper doc comment
    trimmed_span: Span,
    /// expected variant
    variant: CommentVariant,
}

fn comment_variant_span_range_validation(index: usize) {
    let test_data: &[Triplet] = &[
        // 0
        Triplet {
            source: r#"
/// One Doc
struct One;
"#,
            extracted: r#"" One Doc""#,
            trimmed: " One Doc",
            extracted_span: Span {
                start: LineColumn {
                    line: 2_usize,
                    column: 0,
                },
                end: LineColumn {
                    line: 2_usize,
                    column: 10_usize,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 2_usize,
                    column: 3_usize,
                },
                end: LineColumn {
                    line: 2_usize,
                    column: 10_usize,
                },
            },
            variant: CommentVariant::TripleSlash,
        },
        // 1
        Triplet {
            source: r##"
    ///meanie
struct Meanie;
"##,
            extracted: r#""meanie""#,
            trimmed: "meanie",
            extracted_span: Span {
                start: LineColumn {
                    line: 2_usize,
                    column: 0_usize + 7 - PREFIX_RAW_LEN,
                },
                end: LineColumn {
                    line: 2_usize,
                    column: 0_usize + 12 + SUFFIX_RAW_LEN,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 2_usize,
                    column: 0_usize + 7,
                },
                end: LineColumn {
                    line: 2_usize,
                    column: 0_usize + 12,
                },
            },
            variant: CommentVariant::TripleSlash,
        },
        // 2
        Triplet {
            source: r#"
#[doc = "Two Doc"]
struct Two;
"#,
            extracted: r#""Two Doc""#,
            trimmed: "Two Doc",
            extracted_span: Span {
                start: LineColumn {
                    line: 2_usize,
                    column: 0_usize + 10 - GAENSEFUESSCHEN,
                },
                end: LineColumn {
                    line: 2__usize,
                    column: 6__usize + 10 + GAENSEFUESSCHEN,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 2__usize,
                    column: 0__usize + 9,
                },
                end: LineColumn {
                    line: 2__usize,
                    column: 6__usize + 9,
                },
            },
            variant: CommentVariant::MacroDocEqStr("#[doc = ".to_string(), 0),
        },
        // 3
        Triplet {
            source: r##"
    #[doc=r#"Three Doc"#]
struct Three;
"##,
            extracted: r###"r#"Three Doc"#"###,
            trimmed: "Three Doc",
            extracted_span: Span {
                start: LineColumn {
                    line: 2_usize,
                    column: 4__usize + 11,
                },
                end: LineColumn {
                    line: 2__usize,
                    column: 13__usize + 11,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 2__usize,
                    column: 0__usize + 13,
                },
                end: LineColumn {
                    line: 2__usize,
                    column: 13__usize + 8,
                },
            },
            variant: CommentVariant::MacroDocEqStr("#[doc=".to_string(), 2),
        },
        // 4
        Triplet {
            source: r###"
#[doc = r##"Four
has
multiple
lines
"##]
struct Four;
"###,
            extracted: r###"r##"Four
has
multiple
lines
"##"###,
            trimmed: r#"Four
has
multiple
lines
"#,
            extracted_span: Span {
                start: LineColumn {
                    line: 2__usize,
                    column: 12__usize - 4,
                },
                end: LineColumn {
                    line: 6__usize,
                    column: 0__usize + 3,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 2__usize,
                    column: 12__usize,
                },
                end: LineColumn {
                    line: 6__usize,
                    column: 0__usize,
                },
            },
            variant: CommentVariant::MacroDocEqStr("#[doc = ".to_string(), 3),
        },
        // 5
        Triplet {
            source: r###"
#[doc        ="XYZ"]
struct Five;
"###,
            extracted: r#""XYZ""#,
            trimmed: r#"XYZ"#,
            extracted_span: Span {
                start: LineColumn {
                    line: 2__usize,
                    column: 15__usize - GAENSEFUESSCHEN,
                },
                end: LineColumn {
                    line: 2__usize,
                    column: 15__usize + 2 + GAENSEFUESSCHEN,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 2__usize,
                    column: 15_usize,
                },
                end: LineColumn {
                    line: 2_usize,
                    column: 15_usize + 2,
                },
            },
            variant: CommentVariant::MacroDocEqStr("#[doc        =".to_string(), 0),
        },
        // 6
        Triplet {
            source: r#"

    /// if a layer is provided a identiacla "input" and "output", it will only be supplied an
    fn compute_in_place(&self) -> bool {
        false
    }

"#,
            extracted: r#"" if a layer is provided a identiacla "input" and "output", it will only be supplied an""#,
            trimmed: r#" if a layer is provided a identiacla "input" and "output", it will only be supplied an"#,
            extracted_span: Span {
                start: LineColumn {
                    line: 3_usize,
                    column: 7_usize - GAENSEFUESSCHEN,
                },
                end: LineColumn {
                    line: 2_usize,
                    column: 92_usize + GAENSEFUESSCHEN,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 3_usize,
                    column: 7_usize,
                },
                end: LineColumn {
                    line: 3_usize,
                    column: 92_usize,
                },
            },
            variant: CommentVariant::TripleSlash,
        },
        // 7
        Triplet {
            source: r#"

/// 🍉 ← αA<sup>OP</sup>x + βy
fn unicode(&self) -> bool {
    true
}

"#,
            extracted: r#"" 🍉 ← αA<sup>OP</sup>x + βy""#,
            trimmed: r#" 🍉 ← αA<sup>OP</sup>x + βy"#,
            extracted_span: Span {
                start: LineColumn {
                    line: 3_usize,
                    column: 3_usize - GAENSEFUESSCHEN,
                },
                end: LineColumn {
                    line: 2_usize,
                    column: 28_usize + GAENSEFUESSCHEN,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 3_usize,
                    column: 3_usize,
                },
                end: LineColumn {
                    line: 3_usize,
                    column: 28_usize,
                },
            },
            variant: CommentVariant::TripleSlash,
        },
        // 8
        Triplet {
            source: r###"
        #[doc = r##"Four
        has

        multiple
        lines
        "##]
        struct Four;
        "###,
            extracted: r###"r##"Four
        has

        multiple
        lines
        "##"###,
            trimmed: r#"Four
        has

        multiple
        lines
        "#,
            extracted_span: Span {
                start: LineColumn {
                    line: 2_usize,
                    column: 12_usize - 4,
                },
                end: LineColumn {
                    line: 6_usize,
                    column: 0_usize + 3,
                },
            },
            trimmed_span: Span {
                start: LineColumn {
                    line: 2_usize,
                    column: 12_usize,
                },
                end: LineColumn {
                    line: 6_usize,
                    column: 0_usize,
                },
            },
            variant: CommentVariant::MacroDocEqStr("#[ doc = ".to_string(), 3),
        },
    ];

    let _ = env_logger::builder()
        .filter(None, log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    let triplet = test_data[index].clone();
    let literals = annotated_literals(triplet.source);

    assert_eq!(literals.len(), 1);

    let literal = literals.first().expect("Must contain exactly one literal");

    assert_eq!(literal.as_str(), triplet.trimmed);

    // just for better visual errors
    let excerpt = load_span_from(triplet.source.as_bytes(), literal.span()).unwrap();
    let expected_excerpt = load_span_from(triplet.source.as_bytes(), triplet.trimmed_span).unwrap();
    assert_eq!(excerpt, expected_excerpt);

    assert_eq!(literal.span(), triplet.trimmed_span);

    assert_eq!(literal.variant(), triplet.variant);
}

#[test]
fn raw_variant_0_triple_slash() {
    comment_variant_span_range_validation(0);
}

#[test]
fn raw_variant_1_spaces_triple_slash() {
    comment_variant_span_range_validation(1);
}

#[test]
fn raw_variant_2_spaces_doc_eq_single_quote() {
    comment_variant_span_range_validation(2);
}

#[test]
fn raw_variant_3_doc_eq_single_r_hash_quote() {
    comment_variant_span_range_validation(3);
}

#[test]
fn raw_variant_4_doc_eq_multi() {
    comment_variant_span_range_validation(4);
}

#[test]
fn raw_variant_5_doc_spaces_eq_single_quote() {
    comment_variant_span_range_validation(5);
}

#[test]
fn raw_variant_6_quote_chars() {
    comment_variant_span_range_validation(6);
}

#[test]
fn raw_variant_7_unicode_symbols() {
    comment_variant_span_range_validation(7);
}

#[test]
fn variant_to_string() {
    let variant = CommentVariant::MacroDocEqStr("#[ doc = ".to_string(), 0);
    assert_eq!(variant.prefix_string(), r###"#[ doc = ""###);
    let variant = CommentVariant::MacroDocEqStr("#[doc = ".to_string(), 1);
    assert_eq!(variant.prefix_string(), r###"#[doc = r""###);
    let variant = CommentVariant::MacroDocEqStr("#[ doc =".to_string(), 2);
    assert_eq!(variant.prefix_string(), r###"#[ doc =r#""###);
    let variant = CommentVariant::MacroDocEqStr("#[doc=".to_string(), 3);
    assert_eq!(variant.prefix_string(), r###"#[doc=r##""###);
}

#[test]
fn variant_suffix_string() {
    let variant = CommentVariant::MacroDocEqStr("#[ doc= ".to_string(), 0);
    assert_eq!(variant.suffix_string(), r###""]"###);
    let variant = CommentVariant::MacroDocEqStr("#[doc = ".to_string(), 1);
    assert_eq!(variant.suffix_string(), r###""]"###);
    let variant = CommentVariant::MacroDocEqStr("#[doc = ".to_string(), 2);
    assert_eq!(variant.suffix_string(), r###""#]"###);
    let variant = CommentVariant::MacroDocEqStr("#[ doc =".to_string(), 3);
    assert_eq!(variant.suffix_string(), r###""##]"###);
}

#[test]
fn variant_consistency() {
    let variants = vec![
        CommentVariant::TripleSlash,
        CommentVariant::DoubleSlashEM,
        CommentVariant::CommonMark,
        CommentVariant::MacroDocEqStr("#[ doc= ".to_string(), 0),
        CommentVariant::MacroDocEqStr("#[doc = ".to_string(), 1),
        CommentVariant::MacroDocEqStr("#[doc = ".to_string(), 2),
        CommentVariant::MacroDocEqStr("#[ doc     =".to_string(), 3),
    ];

    for variant in variants {
        let variant = dbg!(variant);
        assert_eq!(
            variant.prefix_string().chars().count(),
            variant.prefix_len()
        );
        assert_eq!(
            variant.suffix_string().chars().count(),
            variant.suffix_len()
        );
    }
}
