use super::DocAssignment;
use crate::{CheckableChunk, Clusters, ExtractionWarningKind};

#[test]
fn metadata_envelopes_keep_unrelated_payloads_opaque() {
    let source = r#"#[unsafe(no_mangle)]
#[clippy::audit[doc = "Bracket decoy."]]
#[rustfmt::skip{doc = "Brace decoy."}]
#[clippy::hint = |packet, route| packet]
#[cfg_attr(all(), clippy::hint = inspect::<u8, u16>(), r#doc = "Packet guide.")]
struct Packet;"#;
    let clusters = Clusters::load_from_str(source, true, false).unwrap();
    assert!(clusters.warnings().is_empty());
    let chunks = Vec::<CheckableChunk>::from(clusters);
    let markdown = Vec::from_iter(chunks.iter().map(CheckableChunk::as_str));
    assert_eq!(markdown, [r#"Packet guide."#]);
}

#[test]
fn doc_expressions_keep_their_existing_classification() {
    let source = r#"#[cfg_attr(remote, doc = -1)]
#[doc = "Local guide."]
struct Local;
#[doc = #[allow(dead_code)] "Attributed guide."]
struct Attributed;"#;
    let clusters = Clusters::load_from_str(source, true, false).unwrap();
    let warning_sources = Vec::from_iter(clusters.warnings().iter().map(|warning| {
        assert_eq!(warning.kind, ExtractionWarningKind::UnsupportedAttribute);
        crate::util::load_span_from(source.as_bytes(), warning.span).unwrap()
    }));
    assert_eq!(warning_sources, [r#"-1"#]);
    let chunks = Vec::<CheckableChunk>::from(clusters);
    let markdown = Vec::from_iter(chunks.iter().map(CheckableChunk::as_str));
    assert_eq!(markdown, [r#"Local guide."#, r#"Attributed guide."#]);
    assert!(chunks.iter().all(|chunk| !chunk.is_reflow_safe()));
}

#[test]
fn doc_assignment_parse() {
    let _ = syn::parse_str::<DocAssignment>(r########"doc=foo!(bar!(xxx))"########).unwrap();
    let _ = syn::parse_str::<DocAssignment>(r########"doc="s""########).unwrap();
    let _ = syn::parse_str::<DocAssignment>(r########"doc=r#"s"#"########).unwrap();
    let _ = syn::parse_str::<DocAssignment>(r########"doc=r##"s"##"########).unwrap();
    let _ = syn::parse_str::<DocAssignment>(r########"doc=r###"s"###"########).unwrap();
    let _ = syn::parse_str::<DocAssignment>(r########"doc=r####"s"####"########).unwrap();
}
