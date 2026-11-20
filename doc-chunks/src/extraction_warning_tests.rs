use super::*;

/// A warning display base must not alter warning origins or chunk index keys.
#[test]
fn warning_display_base_preserves_stored_origins() {
    let input_root = if cfg!(windows) {
        PathBuf::from(r"C:\workspace")
    } else {
        PathBuf::from("/workspace")
    };
    let input_base = input_root.join("delivery");
    let input_origin =
        ContentOrigin::RustSourceFile(input_root.join("certificates").join("transport.rs"));
    let input_source = r#"/// Secure transport overview.
#[cfg_attr(trace, doc = include_str!("trace-guide.md"))]
struct SecureTransport;"#;
    let expected_origin = input_origin.clone();
    let expected_origins = vec![expected_origin.clone()];
    let expected_warning_count = 1;
    let expected_chunk_count = 1;
    let expected_default_base = None;
    let mut documentation = Documentation::new();
    assert_eq!(documentation.warning_path_base, expected_default_base);
    documentation.set_warning_path_base(input_base);
    documentation
        .add_rust_with_budget(
            input_origin,
            input_source,
            true,
            false,
            CfgBudget::default(),
        )
        .unwrap();

    assert_eq!(documentation.warnings().len(), expected_warning_count);
    assert_eq!(documentation.warnings()[0].origin, expected_origin);
    let actual_origins = Vec::from_iter(documentation.iter().map(|(origin, _)| origin.clone()));
    assert_eq!(actual_origins, expected_origins);
    assert_eq!(
        documentation.get(&expected_origin).unwrap().len(),
        expected_chunk_count
    );
}

/// A caller-selected warning path leaves extracted Markdown and stored origins intact.
#[test]
fn warning_display_path_preserves_documentation() {
    let origin = ContentOrigin::RustSourceFile(PathBuf::from("workspace/src/transport.rs"));
    let display_path = PathBuf::from("src/transport.rs");
    let source = r#"#[doc = "Secure transport overview."]
#[cfg_attr(trace, doc = include_str!("trace-guide.md"))]
struct SecureTransport;"#;
    let mut documentation = Documentation::new();
    documentation.set_warning_path_base(PathBuf::from("workspace"));
    documentation.set_warning_path(display_path.clone());
    documentation
        .add_rust(origin.clone(), source, true, false)
        .unwrap();

    let chunks = documentation.get(&origin).unwrap();
    assert_eq!(chunks[0].as_str(), r"Secure transport overview.");
    assert_eq!(chunks.len(), 1);
    assert_eq!(documentation.warnings().len(), 1);
    assert_eq!(documentation.warnings()[0].origin, origin);
    assert_eq!(documentation.warning_path, Some(display_path));
}

#[test]
fn merging_documentation_retains_warnings_with_empty_and_partial_output() {
    /// Source documentation paired with its retained chunk count and symbolic condition.
    struct Case {
        source: &'static str,
        expected_chunk_count: usize,
        expected_cfg: Option<&'static str>,
    }
    let cases = [
        Case {
            source: r#"#[doc = include_str!("socket-guide.md")]
struct SocketGuide;"#,
            expected_chunk_count: 0,
            expected_cfg: None,
        },
        Case {
            source: r#"/// Secure delivery.
#[cfg_attr(tls, doc = include_str!("certificates.md"))]
struct SecureDelivery;"#,
            expected_chunk_count: 1,
            expected_cfg: Some("not(tls)"),
        },
    ];
    let merge_methods = [Documentation::merge, Documentation::merge_unique];
    let expected_warning_count = 1;
    let expected_kind = ExtractionWarningKind::OpaqueDocumentation;
    let existing_origin = ContentOrigin::RustSourceFile(PathBuf::from("overview.rs"));
    let existing_source = r#"/// Connection overview.
struct ConnectionOverview;"#;
    let expected_existing_chunk_count = 1;
    for Case {
        source,
        expected_chunk_count,
        expected_cfg,
    } in cases
    {
        let incoming_origin = ContentOrigin::RustSourceFile(PathBuf::from("delivery.rs"));
        let incoming = Documentation::load_from_str(incoming_origin.clone(), source, true, false);
        let expected_span = incoming.warnings()[0].warning.span;
        let expected_payload = util::load_span_from(source.as_bytes(), expected_span).unwrap();
        for merge in merge_methods {
            let mut merged =
                Documentation::load_from_str(existing_origin.clone(), existing_source, true, false);
            merge(&mut merged, incoming.clone());
            assert_eq!(merged.warnings().len(), expected_warning_count);
            let warning = &merged.warnings()[0];
            assert_eq!(warning.origin, incoming_origin);
            assert_eq!(warning.warning.kind, expected_kind);
            assert_eq!(warning.warning.span, expected_span);
            let actual_payload =
                util::load_span_from(source.as_bytes(), warning.warning.span).unwrap();
            assert_eq!(actual_payload, expected_payload);
            let chunks = merged.get(&incoming_origin).unwrap();
            assert_eq!(chunks.len(), expected_chunk_count);
            if let Some(chunk) = chunks.first() {
                assert_eq!(chunk.cfg_example(), expected_cfg);
            }
            assert_eq!(
                merged.get(&existing_origin).unwrap().len(),
                expected_existing_chunk_count
            );
        }
    }
}

#[test]
fn unique_merge_keeps_new_warnings_for_existing_origins_without_duplicates() {
    let origin = ContentOrigin::RustSourceFile(PathBuf::from("transport.rs"));
    let original_source = r#"/// Transport overview.
struct Transport;"#;
    let incoming_source = r#"/// Transport overview.
#[cfg_attr(trace, doc = include_str!("trace-guide.md"))]
struct Transport;"#;
    let expected_warning_count = 1;
    let expected_chunk_count = 1;
    let expected_text = " Transport overview.";
    let mut merged = Documentation::load_from_str(origin.clone(), original_source, true, false);
    let incoming = Documentation::load_from_str(origin.clone(), incoming_source, true, false);
    merged.merge_unique(incoming.clone());
    assert_eq!(merged.warnings().len(), expected_warning_count);
    assert_eq!(merged.warnings()[0].origin, origin);
    assert_eq!(merged.get(&origin).unwrap().len(), expected_chunk_count);
    assert_eq!(merged.get(&origin).unwrap()[0].as_str(), expected_text);
    merged.merge_unique(incoming);
    assert_eq!(merged.warnings().len(), expected_warning_count);
    assert_eq!(merged.get(&origin).unwrap().len(), expected_chunk_count);
    assert_eq!(merged.get(&origin).unwrap()[0].as_str(), expected_text);
}

#[test]
fn excerpt_warnings_survive_empty_output_and_use_original_source_coordinates() {
    let source = r#"// Transport examples.
fn fixture() {
    #[cfg(not())]
    /// Socket documentation.
    struct Socket;
}
"#;
    let excerpt = Span {
        start: LineColumn { line: 3, column: 4 },
        end: LineColumn {
            line: 5,
            column: 17,
        },
    };
    let origin = ContentOrigin::RustDocTest(PathBuf::from("transport.rs"), excerpt);
    let expected_count = 1;
    let expected_chunk_count = 0;
    let expected_kind = ExtractionWarningKind::UnsupportedAttribute;
    let expected_start = LineColumn { line: 3, column: 5 };
    let expected_payload = "[cfg(not())]";
    let docs = Documentation::load_from_str(origin.clone(), source, true, false);
    // Losing all chunks must not lose the diagnostic or its original file location.
    assert_eq!(docs.get(&origin).unwrap_or(&[]).len(), expected_chunk_count);
    assert_eq!(docs.warnings().len(), expected_count);
    let located = &docs.warnings()[0];
    assert_eq!(located.origin, origin);
    assert_eq!(located.warning.kind, expected_kind);
    assert_eq!(located.warning.span.start, expected_start);
    let actual_payload = util::load_span_from(source.as_bytes(), located.warning.span).unwrap();
    assert_eq!(actual_payload, expected_payload);
}
