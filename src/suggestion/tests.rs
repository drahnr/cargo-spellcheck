use super::*;
use crate::{CommentVariant, Documentation, LineColumn};

use std::fmt;
use std::path::PathBuf;

/// A test helper comparing the output against an expected output.
///
/// Strips all color codes from both the expected string and the
/// display-able object.
fn assert_display_eq<D: fmt::Display, S: AsRef<str>>(display: D, s: S) {
    let expected = s.as_ref();
    let expected = console::strip_ansi_codes(expected);

    // uses the display impl
    let reality = display.to_string();
    let reality = console::strip_ansi_codes(reality.as_str());
    assert_eq!(reality, expected);
}

/// Relative rendering preserves the origin and the existing doctest line offset.
#[cfg(unix)]
#[test]
fn fmt_paths_preserve_origin_and_doctest_offset() {
    /// Rendering inputs and expected location headers for each origin kind.
    struct RenderingCase {
        source: &'static str,
        origin: ContentOrigin,
        display_path: &'static str,
        typo: &'static str,
        correction: &'static str,
        expected_relative_location: &'static str,
        expected_absolute_location: &'static str,
    }

    let cases = [
        RenderingCase {
            source: r#"/// Stores downloaded artifacts.
/// Reuses the persistant cache.
struct ArtifactCache;"#,
            origin: ContentOrigin::RustSourceFile(PathBuf::from("/workspace/harbor/src/cache.rs")),
            display_path: "src/cache.rs",
            typo: "persistant",
            correction: "persistent",
            expected_relative_location: "  --> src/cache.rs:2",
            expected_absolute_location: "  --> /workspace/harbor/src/cache.rs:2",
        },
        RenderingCase {
            source: r#"/// Sends queued packets.
/// Retries an interupted connection.
struct PacketSender;"#,
            origin: ContentOrigin::RustDocTest(
                PathBuf::from("/workspace/harbor/src/transport.rs"),
                Span {
                    start: LineColumn {
                        line: 40,
                        column: 0,
                    },
                    end: LineColumn {
                        line: 43,
                        column: 0,
                    },
                },
            ),
            display_path: "../src/transport.rs",
            typo: "interupted",
            correction: "interrupted",
            expected_relative_location: "  --> ../src/transport.rs:42",
            expected_absolute_location: "  --> /workspace/harbor/src/transport.rs:42",
        },
    ];

    for case in cases {
        let source = case.source;
        let expected_origin = case.origin.clone();
        let typo = case.typo;
        let correction = case.correction;
        let expected_relative_location = case.expected_relative_location;
        let expected_absolute_location = case.expected_absolute_location;
        let docs = Documentation::load_from_str(ContentOrigin::TestEntityRust, source, true, false);
        let chunks = docs.get(&ContentOrigin::TestEntityRust).unwrap();
        let chunk = chunks
            .iter()
            .find(|chunk| chunk.as_str().contains(typo))
            .expect("documentation containing the typo must be extracted");
        let start = chunk.as_str().find(typo).unwrap();
        let range = start..start + typo.len();
        let mappings = chunk.find_spans(range.clone());
        let (_, span) = mappings.first().unwrap();
        let suggestion = Suggestion {
            detector: Detector::Dummy,
            checked_word: None,
            origin: case.origin,
            chunk,
            span: *span,
            range,
            replacements: vec![correction.to_owned()],
            description: None,
        };

        let relative_display = suggestion.display_with_path(Path::new(case.display_path));
        let relative_rendering = relative_display.to_string();
        let relative_rendering = console::strip_ansi_codes(&relative_rendering);
        let relative_location = relative_rendering.lines().nth(1).unwrap();
        assert_eq!(relative_location, expected_relative_location);
        assert_eq!(suggestion.origin, expected_origin);

        let absolute_rendering = suggestion.to_string();
        let absolute_rendering = console::strip_ansi_codes(&absolute_rendering);
        let absolute_location = absolute_rendering.lines().nth(1).unwrap();
        assert_eq!(absolute_location, expected_absolute_location);
        let expected_absolute_rendering =
            relative_rendering.replace(expected_relative_location, expected_absolute_location);
        assert_eq!(absolute_rendering, expected_absolute_rendering);
        let expected_replacement = format!("   | - {correction}");
        assert!(relative_rendering.contains(&expected_replacement));
    }
}

/// Guarded documentation shows its symbolic cfg on the first diagnostic line.
#[test]
fn fmt_cfg_examples_are_shown_for_guarded_docs() {
    /// Source, spelling correction, and expected symbolic predicate for a guard.
    struct GuardedDocumentationCase {
        source: &'static str,
        expected_example: &'static str,
        typo: &'static str,
        correction: &'static str,
    }

    let cases = [
        GuardedDocumentationCase {
            source: r#"#[cfg(feature = "serde")]
/// Supports serialzation to JSON.
struct SerializableRecord;"#,
            expected_example: r#"feature = "serde""#,
            typo: "serialzation",
            correction: "serialization",
        },
        GuardedDocumentationCase {
            source: r#"#[cfg(not(target_arch = "wasm32"))]
/// Observes files for chagnes.
struct NativeFileWatcher;"#,
            expected_example: r#"not(target_arch = "wasm32")"#,
            typo: "chagnes",
            correction: "changes",
        },
        GuardedDocumentationCase {
            source: r#"#[cfg(all(feature = "simd", target_arch = "x86_64"))]
/// Performs accelarated searches.
struct VectorizedSearch;"#,
            expected_example: r#"all(feature = "simd", target_arch = "x86_64")"#,
            typo: "accelarated",
            correction: "accelerated",
        },
        GuardedDocumentationCase {
            source: r#"#[cfg_attr(docsrs, doc = "See the API referance.")]
struct HostedDocumentation;"#,
            expected_example: "docsrs",
            typo: "referance",
            correction: "reference",
        },
        GuardedDocumentationCase {
            source: r#"#[cfg(all())]
/// Always availble to readers.
struct AlwaysDocumented;"#,
            expected_example: "all()",
            typo: "availble",
            correction: "available",
        },
        GuardedDocumentationCase {
            source: r#"#[cfg(not(unix))]
/// Opens a compatable system handle.
struct PortableHandle;"#,
            expected_example: "not(unix)",
            typo: "compatable",
            correction: "compatible",
        },
    ];

    for case in cases {
        let source = case.source;
        let expected_example = case.expected_example;
        let typo = case.typo;
        let correction = case.correction;
        let docs = Documentation::load_from_str(ContentOrigin::TestEntityRust, source, true, false);
        let chunks = docs.get(&ContentOrigin::TestEntityRust).unwrap();
        let chunk = chunks
            .iter()
            .find(|chunk| chunk.as_str().contains(typo))
            .expect("conditional documentation must be extracted");
        let expected_cfg_example = Some(expected_example);
        assert_eq!(chunk.cfg_example(), expected_cfg_example);
        let start = chunk.as_str().find(typo).unwrap();
        let range = start..start + typo.len();
        let mappings = chunk.find_spans(range.clone());
        let expected_mapping_count = 1;
        assert_eq!(mappings.len(), expected_mapping_count);
        let (mapped_range, span) = mappings.first().unwrap();
        let expected_range = &range;
        assert_eq!(mapped_range, expected_range);

        let (line, text) = source
            .lines()
            .enumerate()
            .find(|(_, text)| text.contains(typo))
            .unwrap();
        let column = text.find(typo).unwrap();
        let expected_span = Span {
            start: LineColumn {
                line: line + 1,
                column,
            },
            end: LineColumn {
                line: line + 1,
                column: column + typo.len() - 1,
            },
        };
        assert_eq!(*span, expected_span);

        let descriptions = [None, Some("Possible spelling mistake found.".to_owned())];
        for description in descriptions {
            let replacement_cases = [vec![], vec![correction.to_owned()]];
            for replacements in replacement_cases {
                let suggestion = Suggestion {
                    detector: Detector::Dummy,
                    checked_word: None,
                    origin: ContentOrigin::TestEntityRust,
                    chunk,
                    range: range.clone(),
                    span: *span,
                    replacements,
                    description: description.clone(),
                };
                let display_path = Path::new("entity.rs");
                let renderings = [
                    suggestion.to_string(),
                    suggestion.display_with_path(display_path).to_string(),
                ];
                let expected_first_line =
                    format!("error: spellcheck(Dummy) with cfg({expected_example})");
                let old_footer = "example cfg (symbolic):";
                let expected_annotation = format!("cfg({expected_example})");
                let expected_annotation_count = 1;
                let expected_replacement = format!(" - {correction}");
                let expected_location_suffix = format!("entity.rs:{}", line + 1);
                for rendering in renderings {
                    let rendered = console::strip_ansi_codes(&rendering);
                    let first_line = rendered.lines().next().unwrap();
                    assert_eq!(first_line, expected_first_line);
                    assert!(!rendered.contains(old_footer), "{rendered}");
                    assert_eq!(
                        rendered.matches(&expected_annotation).count(),
                        expected_annotation_count,
                    );
                    assert!(rendered.contains(&expected_location_suffix), "{rendered}");
                    if let Some(description) = &description {
                        assert!(rendered.contains(description), "{rendered}");
                    }
                    if !suggestion.replacements.is_empty() {
                        assert!(rendered.contains(&expected_replacement), "{rendered}");
                    }
                }
            }
        }
    }
}

/// Forced styling remains local and colors only the cfg annotation.
#[test]
fn fmt_cfg_annotation_is_bold_magenta() {
    let example = "not(unix)";
    let annotation = styled_cfg(example).force_styling(true);
    let rendered = format!("error: spellcheck(Dummy) with {annotation}");
    let expected = "error: spellcheck(Dummy) with \x1b[35m\x1b[1mcfg(not(unix))\x1b[0m";
    assert_eq!(rendered, expected);
}

#[test]
fn fmt_single_line_with_and_without_replacements() {
    let (content, cases) = (
        " Is it dyrck again?",
        [
            (
                vec!["replacement_0", "replacement_1", "replacement_2"],
                r#"error: spellcheck(Dummy)
  --> /tmp/test/entity.rs:1
   |
 1 |  Is it dyrck again?
   |        ^^^^^
   | - replacement_0, replacement_1, or replacement_2
   |
   |   Possible spelling mistake found.
"#,
            ),
            (
                vec![],
                r#"error: spellcheck(Dummy)
  --> /tmp/test/entity.rs:1
   |
 1 |  Is it dyrck again?
   |        ^^^^^
   |   Possible spelling mistake found.
"#,
            ),
        ],
    );
    let chunk = CheckableChunk::from_str(
        content,
        indexmap::indexmap! { 0..18 => Span {
            start: LineColumn { line: 1, column: 0 },
            end: LineColumn { line: 1, column: 17 },
        } },
        CommentVariant::TripleSlash,
    );

    for (replacements, expected) in cases {
        let suggestion = Suggestion {
            detector: Detector::Dummy,
            checked_word: None,
            origin: ContentOrigin::TestEntityRust,
            chunk: &chunk,
            range: 7..12,
            span: Span {
                start: LineColumn { line: 1, column: 6 },
                end: LineColumn {
                    line: 1,
                    column: 10,
                },
            },
            replacements: Vec::from_iter(replacements.into_iter().map(str::to_owned)),
            description: Some("Possible spelling mistake found.".to_owned()),
        };
        assert_display_eq(suggestion, expected);
    }
}

#[test]
fn fmt_1_multi() {
    const CONTENT: &str = r#" Line mitake 1
 Anowher 2
 Last"#;

    let chunk = CheckableChunk::from_str(
        CONTENT,
        indexmap::indexmap! {
            0..13 => Span {
                start: LineColumn {
                    line: 1,
                    column: 4,
                },
                end: LineColumn {
                    line: 1,
                    column: 16,
                }
            },
            14..24 => Span {
                start: LineColumn {
                    line: 2,
                    column: 4,
                },
                end: LineColumn {
                    line: 2,
                    column: 12,
                }
            },
            25..29 => Span {
                start: LineColumn {
                    line: 3,
                    column: 4,
                },
                end: LineColumn {
                    line: 3,
                    column: 7,
                }
            }
        },
        CommentVariant::TripleSlash,
    );

    let suggestion = Suggestion {
        detector: Detector::Dummy,
        checked_word: None,
        origin: ContentOrigin::TestEntityRust,
        chunk: &chunk,
        range: 6..12,
        span: Span {
            start: LineColumn {
                line: 1,
                column: 10,
            },
            end: LineColumn {
                line: 1,
                column: 15,
            },
        },
        replacements: vec![
            "replacement_0".to_owned(),
            "replacement_1".to_owned(),
            "replacement_2".to_owned(),
        ],
        description: Some("Possible spelling mistake found.".to_owned()),
    };

    const EXPECTED: &str = r#"error: spellcheck(Dummy)
  --> /tmp/test/entity.rs:1
   |
 1 |  Line mitake 1
   |       ^^^^^^
   | - replacement_0, replacement_1, or replacement_2
   |
   |   Possible spelling mistake found.
"#;

    assert_display_eq(suggestion, EXPECTED);
}

#[test]
fn fmt_2_multi_80_plus() {
    const CONTENT: &str = r#" Line mitake 1
 Suuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuper duuuuuuuuuuuuuuuuuuuuuuuuper too long
 "#;

    let chunk = CheckableChunk::from_str(
        CONTENT,
        indexmap::indexmap! {
            0..13 => Span {
                start: LineColumn {
                    line: 1,
                    column: 4,
                },
                end: LineColumn {
                    line: 1,
                    column: 16,
                }
            },
            14..101 => Span {
                start: LineColumn {
                    line: 2,
                    column: 4,
                },
                end: LineColumn {
                    line: 2,
                    column: 90,
                }
            }
        },
        CommentVariant::TripleSlash,
    );

    let suggestion = Suggestion {
        detector: Detector::Dummy,
        checked_word: None,
        origin: ContentOrigin::TestEntityRust,
        chunk: &chunk,
        range: 66..94,
        span: Span {
            start: LineColumn { line: 2, column: 5 },
            end: LineColumn {
                line: 2,
                column: 92,
            },
        },
        replacements: vec![
            "replacement_0".to_owned(),
            "replacement_1".to_owned(),
            "replacement_2".to_owned(),
        ],
        description: Some("Possible spelling mistake found.".to_owned()),
    };

    const EXPECTED: &str = r#"error: spellcheck(Dummy)
  --> /tmp/test/entity.rs:2
   |
 2 | ..uuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuper duuu...uper too long
   |                                                 ^^^^^^^^^^^
   | - replacement_0, replacement_1, or replacement_2
   |
   |   Possible spelling mistake found.
"#;

    assert_display_eq(suggestion, EXPECTED);
}

#[test]
fn multiline_is_dbg_printable() {
    let _ = env_logger::builder()
        .is_test(true)
        .filter(None, log::LevelFilter::Trace)
        .try_init();

    use crate::documentation::CheckableChunk;
    let chunk = CheckableChunk::from_str(
        r#"0
2345
7@n"#,
        indexmap::indexmap! { 0..10 => Span {
            start : LineColumn {
                line: 7usize,
                column: 8usize,
            },
            end : LineColumn {
                line: 9usize,
                column: 4usize,
            }
        } },
        CommentVariant::TripleSlash,
    );

    let suggestion = Suggestion {
        detector: Detector::Dummy,
        checked_word: None,
        origin: ContentOrigin::TestEntityRust,
        chunk: &chunk,
        span: Span {
            start: LineColumn {
                line: 8usize,
                column: 0,
            },
            end: LineColumn {
                line: 8usize,
                column: 3,
            },
        },
        range: 2..6,
        replacements: vec!["whocares".to_owned()],
        description: None,
    };

    let suggestion = dbg!(suggestion);

    log::info!("fmt debug=\n{suggestion:?}\n<");
    log::info!("fmt display=\n{suggestion}\n<");
}

/// Inclusive source spans overlap symmetrically only within the same origin.
#[test]
fn overlapped() {
    /// Inputs and expected overlap for inclusive source-coordinate spans.
    struct OverlapCase {
        name: &'static str,
        left: Span,
        right: Span,
        different_origin: bool,
        expected: bool,
    }

    /// Build an inclusive source span from its start and end coordinates.
    fn source_span(
        start_line: usize,
        start_column: usize,
        end_line: usize,
        end_column: usize,
    ) -> Span {
        Span {
            start: LineColumn {
                line: start_line,
                column: start_column,
            },
            end: LineColumn {
                line: end_line,
                column: end_column,
            },
        }
    }
    let chunk = CheckableChunk::from_str(
        r#"0
2345
7@n"#,
        indexmap::indexmap! { 0..10 => Span {
            start : LineColumn {
                line: 7usize,
                column: 8usize,
            },
            end : LineColumn {
                line: 9usize,
                column: 4usize,
            }
        } },
        CommentVariant::TripleSlash,
    );
    let suggestion = Suggestion {
        detector: Detector::Dummy,
        checked_word: None,
        origin: ContentOrigin::TestEntityRust,
        chunk: &chunk,
        span: Span {
            start: LineColumn {
                line: 8usize,
                column: 1,
            },
            end: LineColumn {
                line: 8usize,
                column: 3,
            },
        },
        range: 2..6,
        replacements: vec!["whocares".to_owned()],
        description: None,
    };
    let cases = [
        OverlapCase {
            name: "partial same-line overlap",
            left: source_span(8, 1, 8, 3),
            right: source_span(8, 0, 8, 2),
            different_origin: false,
            expected: true,
        },
        OverlapCase {
            name: "contained same-line span",
            left: source_span(8, 1, 8, 3),
            right: source_span(8, 2, 8, 3),
            different_origin: false,
            expected: true,
        },
        OverlapCase {
            name: "shared endpoint",
            left: source_span(8, 0, 8, 2),
            right: source_span(8, 2, 8, 3),
            different_origin: false,
            expected: true,
        },
        OverlapCase {
            name: "identical one-character spans",
            left: source_span(8, 2, 8, 2),
            right: source_span(8, 2, 8, 2),
            different_origin: false,
            expected: true,
        },
        OverlapCase {
            name: "immediately adjacent spans",
            left: source_span(8, 0, 8, 1),
            right: source_span(8, 2, 8, 3),
            different_origin: false,
            expected: false,
        },
        OverlapCase {
            name: "separated same-line spans",
            left: source_span(8, 0, 8, 0),
            right: source_span(8, 3, 8, 3),
            different_origin: false,
            expected: false,
        },
        OverlapCase {
            name: "multiline containment despite smaller end column",
            left: source_span(7, 8, 9, 4),
            right: source_span(8, 5, 8, 7),
            different_origin: false,
            expected: true,
        },
        OverlapCase {
            name: "crossing multiline spans",
            left: source_span(7, 8, 8, 2),
            right: source_span(8, 1, 9, 4),
            different_origin: false,
            expected: true,
        },
        OverlapCase {
            name: "multiline shared endpoint",
            left: source_span(7, 8, 8, 2),
            right: source_span(8, 2, 9, 4),
            different_origin: false,
            expected: true,
        },
        OverlapCase {
            name: "multiline spans adjacent on shared line",
            left: source_span(7, 8, 8, 1),
            right: source_span(8, 2, 9, 4),
            different_origin: false,
            expected: false,
        },
        OverlapCase {
            name: "disjoint line ranges",
            left: source_span(7, 8, 8, 2),
            right: source_span(9, 0, 10, 4),
            different_origin: false,
            expected: false,
        },
        OverlapCase {
            name: "identical spans in different origins",
            left: source_span(8, 1, 8, 3),
            right: source_span(8, 1, 8, 3),
            different_origin: true,
            expected: false,
        },
    ];

    for case in cases {
        let mut left = suggestion.clone();
        left.span = case.left;
        let mut right = suggestion.clone();
        right.span = case.right;
        if case.different_origin {
            right.origin = ContentOrigin::RustSourceFile(PathBuf::from("src/transport.rs"));
        }
        let expected = case.expected;

        assert_eq!(left.is_overlapped(&right), expected, "{}", case.name);
        assert_eq!(
            right.is_overlapped(&left),
            expected,
            "{} reversed",
            case.name
        );
    }
}

#[test]
fn fmt_with_path_preserves_origin_and_default_display() {
    let span = Span {
        start: LineColumn { line: 1, column: 0 },
        end: LineColumn { line: 1, column: 4 },
    };
    let chunk = CheckableChunk::from_str(
        "typpo",
        indexmap::indexmap! { 0..5 => span },
        CommentVariant::TripleSlash,
    );
    let origin_path = std::env::current_dir().unwrap().join("src").join("lib.rs");
    let display_path = std::path::Path::new("src").join("lib.rs");
    let doctest_span = Span {
        start: LineColumn {
            line: 42,
            column: 0,
        },
        end: LineColumn {
            line: 44,
            column: 0,
        },
    };

    for (origin, line) in [
        (ContentOrigin::RustSourceFile(origin_path.clone()), 1),
        (
            ContentOrigin::RustDocTest(origin_path.clone(), doctest_span),
            43,
        ),
    ] {
        let suggestion = Suggestion {
            detector: Detector::Dummy,
            checked_word: None,
            origin: origin.clone(),
            chunk: &chunk,
            span,
            range: 0..5,
            replacements: vec!["typo".to_owned()],
            description: Some("Possible spelling mistake found.".to_owned()),
        };
        let expected = format!(
            r#"error: spellcheck(Dummy)
  --> {}:{line}
   |
 1 | typpo
   | ^^^^^
   | - typo
   |
   |   Possible spelling mistake found.
"#,
            origin_path.display(),
        );
        let default_display = suggestion.to_string();
        assert_display_eq(&suggestion, &expected);
        assert_display_eq(
            suggestion.display_with_path(&display_path),
            expected.replacen(
                &origin_path.display().to_string(),
                &display_path.display().to_string(),
                1,
            ),
        );
        assert_eq!(suggestion.origin, origin);
        assert_eq!(suggestion.origin.as_path(), origin_path);
        assert_eq!(suggestion.to_string(), default_display);
    }
}
