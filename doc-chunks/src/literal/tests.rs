use super::*;
use crate::testcase::annotated_literals_raw;
use assert_matches::assert_matches;

#[test]
fn trimmed_content_is_empty() {
    for (content, pre, post, expected_empty) in [("/**/", 2, 2, true), ("/*🌡*/", 2, 2, false)] {
        let literal =
            TrimmedLiteral::from(CommentVariant::SlashAsterisk, content, pre, post, 1, 0).unwrap();
        assert_eq!(literal.is_empty(), expected_empty);
        assert_eq!(literal.is_empty(), literal.as_str().is_empty());
        assert_eq!(literal.len(), literal.as_str().len());
    }
}

fn assert_chunk(
    source: &str,
    chunk: &crate::CheckableChunk,
    expected: &str,
    fragments: &[(crate::Range, Span)],
) {
    assert_eq!(chunk.as_str(), expected, "source: {source:?}");
    let actual: Vec<_> = chunk
        .iter()
        .map(|(range, span)| (range.clone(), *span))
        .collect();
    assert_eq!(actual, fragments, "source: {source:?}");
    for (range, span) in fragments {
        // Empty fragments use a reversed inclusive span, not source text.
        if range.is_empty() {
            continue;
        }
        assert_eq!(
            util::load_span_from(source.as_bytes(), *span).unwrap(),
            sub_chars(chunk.as_str(), range.clone()),
        );
        assert_eq!(
            chunk
                .find_spans(range.clone())
                .into_iter()
                .collect::<Vec<_>>(),
            vec![(range.clone(), *span)],
        );
    }
}

fn line_span(line: usize, start: usize, end: usize) -> Span {
    Span {
        start: LineColumn {
            line,
            column: start,
        },
        end: LineColumn { line, column: end },
    }
}

fn doc_chunks(source: &str) -> Vec<crate::CheckableChunk> {
    crate::Clusters::load_from_str(source, true, false)
        .unwrap()
        .into()
}

#[test]
fn line_comment_endings_preserve_exact_content_and_spans() {
    for prefix in ["///", "//!"] {
        for first_ending in ["\n", "\r\n"] {
            for second_ending in ["\n", "\r\n"] {
                for eof_ending in ["", "\n", "\r\n"] {
                    let source = format!(
                            "{prefix} Préface 🦀.{first_ending}{prefix}{second_ending}{prefix} Last.  {eof_ending}"
                        );
                    let chunks = doc_chunks(&source);
                    assert_eq!(chunks.len(), 1);
                    assert_eq!(chunks[0].preferred_line_delimiter(), Some(first_ending));
                    assert_chunk(
                        &source,
                        &chunks[0],
                        " Préface 🦀.\n\n Last.  ",
                        &[
                            (0..11, line_span(1, 3, 13)),
                            (12..12, line_span(2, 3, 2)),
                            (13..21, line_span(3, 3, 10)),
                        ],
                    );
                }
            }
        }
    }
}

#[test]
fn eof_line_comment_after_code_has_no_physical_terminator() {
    for ending in ["\n", "\r\n"] {
        for eof_ending in ["", "\n", "\r\n"] {
            let source = format!("struct X;{ending}/// last{eof_ending}");
            let chunks = doc_chunks(&source);
            assert_eq!(chunks.len(), 1);
            let preferred = if eof_ending.is_empty() {
                ending
            } else {
                eof_ending
            };
            assert_eq!(chunks[0].preferred_line_delimiter(), Some(preferred));
            assert_chunk(&source, &chunks[0], " last", &[(0..5, line_span(2, 3, 7))]);
        }
    }
}

#[test]
fn empty_inner_block_comment_does_not_discard_valid_documentation() {
    for ending in ["\n", "\r\n"] {
        let source = format!("/*!*/{ending}//! Module.{ending}struct X;");
        let chunks = doc_chunks(&source);
        assert_eq!(chunks.len(), 1);
        for chunk in &chunks {
            assert_eq!(chunk.preferred_line_delimiter(), Some(ending));
        }
        assert_chunk(
            &source,
            &chunks[0],
            "\n Module.",
            &[(0..0, line_span(1, 3, 2)), (1..9, line_span(2, 3, 10))],
        );
    }
    for ending in ["", "\n", "\r\n"] {
        let source = format!("/*!*/{ending}");
        let chunks = doc_chunks(&source);
        assert_eq!(chunks.len(), 1);
        assert_chunk(&source, &chunks[0], "", &[(0..0, line_span(1, 3, 2))]);
        assert_eq!(
            chunks[0].preferred_line_delimiter(),
            if ending.is_empty() {
                None
            } else {
                Some(ending)
            },
        );
    }
}

#[test]
fn crlf_string_literals_keep_escapes_and_multiline_source_spans() {
    let source = "#[doc = \"one\\ntwo\"]\r\nstruct X;\r\n";
    let chunks = doc_chunks(source);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].preferred_line_delimiter(), Some("\r\n"));
    assert_chunk(
        source,
        &chunks[0],
        "one\\ntwo",
        &[(0..8, line_span(1, 9, 16))],
    );

    let source = "#[doc = r#\"Café\r\n🦀\"#]\r\nstruct X;\r\n";
    let chunks = doc_chunks(source);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].preferred_line_delimiter(), Some("\r\n"));
    assert_chunk(
        source,
        &chunks[0],
        "Café\r\n🦀",
        &[(
            0..7,
            Span {
                start: LineColumn {
                    line: 1,
                    column: 11,
                },
                end: LineColumn { line: 2, column: 0 },
            },
        )],
    );
}

#[test]
fn single_line_comments_keep_newline_metadata_without_content_newlines() {
    let mut literals = Vec::new();
    for ending in ["", "\n", "\r\n"] {
        let source = format!("/// Words that need wrapping.{ending}");
        let literal = annotated_literals_raw(&source).next().unwrap();
        let literal = TrimmedLiteral::load_from(&source, Span::from(literal.span())).unwrap();
        let preferred = if ending.is_empty() {
            None
        } else {
            Some(ending)
        };
        assert_eq!(literal.preferred_line_delimiter(), preferred);
        assert_eq!(literal.clone().preferred_line_delimiter(), preferred);
        let chunk = crate::LiteralSet::from(literal.clone()).into_chunk();
        assert_eq!(chunk.preferred_line_delimiter(), preferred);
        assert_eq!(chunk.clone().preferred_line_delimiter(), preferred);
        assert_chunk(
            &source,
            &chunk,
            " Words that need wrapping.",
            &[(0..26, line_span(1, 3, 28))],
        );
        literals.push(literal);
    }
    assert_ne!(literals[0], literals[1]);
    assert_ne!(literals[1], literals[2]);
}

#[test]
fn eof_comment_inherits_nearest_not_majority_source_delimiter() {
    let source = "struct A;\r\nstruct B;\r\nstruct C;\n/// Last.";
    let chunks = doc_chunks(source);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].preferred_line_delimiter(), Some("\n"));
    assert_chunk(source, &chunks[0], " Last.", &[(0..6, line_span(4, 3, 8))]);
}

#[test]
fn conditional_chunks_prefer_the_first_selected_literals_source_style() {
    struct ExpectedChunk {
        markdown: &'static str,
        delimiter: &'static str,
        fragments: Vec<(crate::Range, Span)>,
    }

    let (source, expected) = (
        // Keep the mixed line endings explicit so editors cannot normalize them.
        r#"#[cfg_attr(flag, doc = "Conditional.")]
/// Shared.
struct Harbor;"#
            .replacen('\n', "\r\n", 1),
        [
            ExpectedChunk {
                markdown: r" Shared.",
                delimiter: "\n",
                fragments: vec![(0..8, line_span(2, 3, 10))],
            },
            ExpectedChunk {
                markdown: r"Conditional.
 Shared.",
                delimiter: "\r\n",
                fragments: vec![(0..12, line_span(1, 24, 35)), (13..21, line_span(2, 3, 10))],
            },
        ],
    );

    let chunks = doc_chunks(&source);
    assert_eq!(chunks.len(), expected.len());
    for expected in expected {
        let chunk = chunks
            .iter()
            .find(|chunk| chunk.as_str() == expected.markdown)
            .unwrap_or_else(|| panic!("missing projection: {:?}", expected.markdown));
        assert_chunk(&source, chunk, expected.markdown, &expected.fragments);
        assert_eq!(chunk.preferred_line_delimiter(), Some(expected.delimiter));
    }
}

#[test]
fn variant_detect() {
    let content = r###"#[doc=r"foo"]"###.to_owned();
    let rendered = r##"r"foo""##.to_owned();
    assert_matches!(
    detect_comment_variant(content.as_str(), &rendered, Span{
        start: LineColumn {
            line: 1,
            column: 6,
        },
        end: LineColumn {
            line: 1,
            column: 12 + 1,
        },
    }), Ok((CommentVariant::MacroDocEqStr(prefix, n_pounds), _, _, _)) => {
        assert_eq!(n_pounds, 1);
        assert_eq!(prefix, "#[doc=");
    });
}

macro_rules! block_comment_test {
    ($name:ident, $content:literal) => {
        #[test]
        fn $name() {
            const CONTENT: &str = $content;
            let mut literals = annotated_literals_raw(CONTENT);
            let literal = literals.next().unwrap();
            assert!(literals.next().is_none());

            let tl = TrimmedLiteral::load_from(CONTENT, Span::from(literal.span())).unwrap();
            assert!(CONTENT.starts_with(tl.prefix()));
            assert!(CONTENT.ends_with(tl.suffix()));
            assert_eq!(
                CONTENT
                    .chars()
                    .skip(tl.pre())
                    .take(tl.len_in_chars())
                    .collect::<String>(),
                tl.as_str().to_owned()
            )
        }
    };
}

block_comment_test!(trimmed_oneline_doc, "/** dooc */");
block_comment_test!(trimmed_oneline_mod, "/*! dooc */");

block_comment_test!(
    trimmed_multi_doc,
    "/**
mood
*/"
);
block_comment_test!(
    trimmed_multi_mod,
    "/*!
mood
*/"
);
