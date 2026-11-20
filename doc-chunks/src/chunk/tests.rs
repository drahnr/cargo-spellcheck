use super::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[test]
fn direct_constructors_preserve_original_content_delimiters() {
    for (content, expected) in [
        ("Words", None),
        ("Words\n", Some("\n")),
        ("Words\r\n", Some("\r\n")),
    ] {
        let chunk = CheckableChunk::from_str(content, IndexMap::new(), CommentVariant::CommonMark);
        assert_eq!(chunk.as_str(), content);
        assert_eq!(chunk.preferred_line_delimiter(), expected);
        assert_eq!(chunk.clone().preferred_line_delimiter(), expected);
    }
}

#[test]
fn source_delimiter_metadata_participates_in_equality_and_hashing() {
    let lf = CheckableChunk::from_str("Words\n", IndexMap::new(), CommentVariant::TripleSlash);
    let mut crlf = lf.clone();
    crlf.set_preferred_line_delimiter(Some("\r\n"));
    assert_ne!(lf, crlf);
    assert_eq!(lf.as_str(), crlf.as_str());
    let mut lf_hash = DefaultHasher::new();
    lf.hash(&mut lf_hash);
    let mut crlf_hash = DefaultHasher::new();
    crlf.hash(&mut crlf_hash);
    assert_ne!(lf_hash.finish(), crlf_hash.finish());
}
