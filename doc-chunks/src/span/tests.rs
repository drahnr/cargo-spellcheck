use super::*;

use crate::literalset::testhelper::gen_literal_set;
use crate::util::load_span_from;
use crate::{chyrp_dbg, chyrp_up, fluff_up};
use crate::{LineColumn, Range, Span};

/// Check reverse mapping of nonempty fragments sharing one source line.
fn assert_source_fragment_ranges(source: &str, expected_content: &str) {
    let clusters = crate::Clusters::load_from_str(source, true, false).unwrap();
    let chunks = Vec::<CheckableChunk>::from(clusters);
    let expected_chunk_count = 1;
    let expected_fragment_count = 2;
    assert_eq!(chunks.len(), expected_chunk_count, "{source}");
    let chunk = &chunks[0];
    assert_eq!(chunk.as_str(), expected_content);
    assert_eq!(chunk.fragment_count(), expected_fragment_count);

    for (expected_range, span) in chunk.iter().filter(|(range, _)| !range.is_empty()) {
        assert_eq!(
            span.to_content_range(chunk).unwrap(),
            *expected_range,
            "{source}: {span:?}"
        );
    }
}

#[test]
fn span_to_range_same_line_docs() {
    let source = r#"#[doc="Harbor"] #[doc="Navigation"] struct Chart;"#;
    let expected_content = "Harbor\nNavigation";
    assert_source_fragment_ranges(source, expected_content);
}

#[test]
fn span_to_range_same_line_one_char_docs() {
    let source = r#"#[doc="N"] #[doc="E"] struct Bearing;"#;
    let expected_content = "N\nE";
    assert_source_fragment_ranges(source, expected_content);
}

#[test]
fn span_to_range_same_line_empty_doc_first() {
    let source = r#"#[doc=""] #[doc="Latitude"] struct Position;"#;
    let expected_content = "\nLatitude";
    assert_source_fragment_ranges(source, expected_content);
}

#[test]
fn span_to_range_same_line_empty_cfg_attr_doc_first() {
    let source = r#"#[cfg_attr(all(), doc="", doc="Forecast")] struct Weather;"#;
    let expected_content = "\nForecast";
    assert_source_fragment_ranges(source, expected_content);
}

#[test]
fn span_to_range_singleline() {
    let _ = env_logger::builder()
        .is_test(true)
        .filter(None, log::LevelFilter::Trace)
        .try_init();

    const CONTENT: &str = fluff_up!("Itsyou!!", " ", "Game-Over!!", " ");
    let set = gen_literal_set(CONTENT);
    let chunk = dbg!(CheckableChunk::from_literalset(set));

    // assuming a `///<space>` comment
    const TRIPPLE_SLASH_PLUS_SPACE: usize = 4;

    // within a file
    const INPUTS: &[Span] = &[
        Span {
            start: LineColumn {
                line: 1usize,
                column: 3usize + TRIPPLE_SLASH_PLUS_SPACE,
            },
            end: LineColumn {
                line: 1usize,
                column: 7usize + TRIPPLE_SLASH_PLUS_SPACE,
            },
        },
        Span {
            start: LineColumn {
                line: 3usize,
                column: 0usize + TRIPPLE_SLASH_PLUS_SPACE,
            },
            end: LineColumn {
                line: 3usize,
                column: 8usize + TRIPPLE_SLASH_PLUS_SPACE,
            },
        },
    ];

    // ranges to be used with `chunk.as_str()`
    // remember that ///<space> counts towards the range!
    // and that newlines are also one char
    const EXPECTED_RANGE: &[Range] = &[4..9, 14..23];

    // note that this may only be single lines, since `///` implies separate literals
    // and as such multiple spans
    const FRAGMENT_STR: &[&str] = &["you!!", "Game-Over"];

    for (input, expected, fragment) in itertools::cons_tuples(
        INPUTS
            .iter()
            .zip(EXPECTED_RANGE.iter())
            .zip(FRAGMENT_STR.iter()),
    ) {
        log::trace!(
            ">>>>>>>>>>>>>>>>\ninput: {input:?}\nexpected: {expected:?}\nfragment:>{fragment}<",
        );
        let range = input
            .to_content_range(&chunk)
            .expect("Inputs are sane, conversion must work.");
        assert_eq!(range, *expected);
        // make sure the span covers what we expect it to cover
        assert_eq!(
            load_span_from(CONTENT.as_bytes(), *input).unwrap(),
            fragment.to_owned()
        );
        assert_eq!(&(&chunk.as_str()[range]), fragment);
    }
}

#[test]
fn span_to_range_multiline() {
    let _ = env_logger::builder()
        .is_test(true)
        .filter(None, log::LevelFilter::Trace)
        .try_init();

    chyrp_dbg!("Xy fff?? Not.., you again!", "", "AlphaOmega", "");
    const CONTENT: &str = chyrp_up!("Xy fff?? Not.., you again!", "", "AlphaOmega", "");
    let set = gen_literal_set(dbg!(CONTENT));
    let chunk = dbg!(CheckableChunk::from_literalset(set));

    // assuming a `#[doc=r#"` comment
    const HASH_BRACKET_DOC_EQ_RAW_HASH_QUOTE: usize = 9;
    // const QUOTE_HASH: usize = 2;

    // within a file
    const INPUTS: &[Span] = &[
        // full
        Span {
            start: LineColumn {
                line: 1usize,
                column: 0usize + HASH_BRACKET_DOC_EQ_RAW_HASH_QUOTE,
            },
            end: LineColumn {
                line: 3usize,
                column: 10usize,
            },
        },
        // sub
        Span {
            start: LineColumn {
                line: 1usize,
                column: 3usize + HASH_BRACKET_DOC_EQ_RAW_HASH_QUOTE,
            },
            end: LineColumn {
                line: 1usize,
                column: 7usize + HASH_BRACKET_DOC_EQ_RAW_HASH_QUOTE,
            },
        },
        Span {
            start: LineColumn {
                line: 3usize,
                column: 0usize,
            },
            end: LineColumn {
                line: 3usize,
                column: 4usize,
            },
        },
    ];

    const EXPECTED_RANGE: &[Range] = &[0..(26 + 1 + 0 + 1 + 10 + 1), 3..8, 28..33];

    const FRAGMENT_STR: &[&str] = &[
        r#"Xy fff?? Not.., you again!

AlphaOmega
"#,
        "fff??",
        "Alpha",
    ];

    for (input, expected, fragment) in itertools::cons_tuples(
        INPUTS
            .iter()
            .zip(EXPECTED_RANGE.iter())
            .zip(FRAGMENT_STR.iter()),
    ) {
        log::trace!(
            ">>>>>>>>>>>>>>>>\ninput: {input:?}\nexpected: {expected:?}\nfragment:>{fragment}<",
        );

        let range = dbg!(input)
            .to_content_range(&chunk)
            .expect("Inputs are sane, conversion must work. qed");
        assert_eq!(range, *expected);

        assert_eq!(
            load_span_from(CONTENT.as_bytes(), *input).unwrap(),
            fragment.to_owned()
        );
        assert_eq!(&(&chunk.as_str()[range]), fragment);
    }
}

#[test]
fn extraction_fluff() {
    const CHUNK_S: &str = r#" one
 two
 three"#;
    const FRAGMENT_SPAN: Span = Span {
        start: LineColumn { line: 1, column: 3 },
        end: LineColumn { line: 1, column: 6 },
    };
    const FRAGMENT_RANGE: Range = 0..4;

    const FRAGMENT_SUB_SPAN: Span = Span {
        start: LineColumn { line: 1, column: 5 },
        end: LineColumn { line: 1, column: 6 },
    };
    let range = dbg!(extract_sub_range_from_span(
        CHUNK_S,
        FRAGMENT_SPAN,
        FRAGMENT_RANGE,
        FRAGMENT_SUB_SPAN,
    )
    .expect("Must be able to extract trivial sub span"));
    assert_eq!(&CHUNK_S[dbg!(range.clone())], "ne");
    assert_eq!(range, 2..4);
}

#[test]
fn extraction_chyrp() {
    const CHUNK_S: &str = r#"one
two
three"#;
    const FRAGMENT_SPAN: Span = Span {
        start: LineColumn {
            line: 1,
            column: 9 + 2,
        },
        end: LineColumn { line: 3, column: 5 },
    };
    const FRAGMENT_RANGE: Range = 0..(3 + 1 + 3 + 5);

    {
        const FRAGMENT_SUB_SPAN: Span = Span {
            start: LineColumn {
                line: 1,
                column: 9 + 2 + 1,
            },
            end: LineColumn {
                line: 1,
                column: 9 + 2 + 1 + 1,
            },
        };
        let range = dbg!(extract_sub_range_from_span(
            CHUNK_S,
            FRAGMENT_SPAN,
            FRAGMENT_RANGE,
            FRAGMENT_SUB_SPAN,
        )
        .expect("Must be able to extract trivial sub span"));
        assert_eq!(&CHUNK_S[dbg!(range.clone())], "ne");
        assert_eq!(range, 1..3);
    }
    {
        const FRAGMENT_SUB_SPAN: Span = Span {
            start: LineColumn { line: 2, column: 1 },
            end: LineColumn { line: 2, column: 2 },
        };
        let range = dbg!(extract_sub_range_from_span(
            CHUNK_S,
            FRAGMENT_SPAN,
            FRAGMENT_RANGE,
            FRAGMENT_SUB_SPAN,
        )
        .expect("Must be able to extract trivial sub span"));
        assert_eq!(&CHUNK_S[dbg!(range.clone())], "wo");
        assert_eq!(range, 5..7);
    }
}
