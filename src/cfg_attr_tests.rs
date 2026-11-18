//! Executable examples of structural, host-independent documentation projections.

use super::{CheckableChunk, ContentOrigin, Documentation};
use crate::util::{load_span_from, sub_chars};

fn chunks(source: &str) -> Vec<CheckableChunk> {
    let docs = Documentation::load_from_str(ContentOrigin::TestEntityRust, source, true, false);
    docs.get(&ContentOrigin::TestEntityRust)
        .unwrap_or(&[])
        .to_vec()
}

fn plain_texts(chunks: &[CheckableChunk]) -> Vec<String> {
    let mut texts: Vec<_> = chunks
        .iter()
        // Whitespace is not relevant to these projection examples; prose order is.
        .map(|chunk| {
            chunk
                .erase_cmark(&Default::default())
                .as_str()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    texts.sort();
    texts
}

fn assert_plain(source: &str, expected: &[&str]) -> Vec<CheckableChunk> {
    let chunks = chunks(source);
    let mut expected: Vec<_> = expected.iter().map(|text| text.to_string()).collect();
    expected.sort();
    // Deliberately do not deduplicate: equal text can have different source identities.
    assert_eq!(plain_texts(&chunks), expected, "source:\n{source}");
    assert_span_roundtrip(source, &chunks);
    chunks
}

fn assert_span_roundtrip(source: &str, chunks: &[CheckableChunk]) {
    for chunk in chunks {
        let mut nonempty = 0;
        for (range, span) in chunk.iter() {
            if range.is_empty() {
                continue;
            }
            nonempty += 1;
            assert_eq!(span.to_content_range(chunk).unwrap(), *range);
            assert_eq!(
                load_span_from(source.as_bytes(), *span).expect("valid source span"),
                sub_chars(chunk.as_str(), range.clone()),
                "range {range:?}, span {span:?}, chunk {:?}",
                chunk.as_str(),
            );
        }
        assert!(
            nonempty > 0,
            "a selected projection must map to source text"
        );
    }
}

#[test]
fn review_raw_attribute_names_preserve_all_documentation_versions() {
    for doc in ["doc", "r#doc"] {
        for cfg_attr in ["cfg_attr", "r#cfg_attr"] {
            let source = format!(
                "#[{doc} = \"Anchor.\"]\n\
                 #[{cfg_attr}(flag, {doc} = \"Selected.\")]\n\
                 #[{cfg_attr}(not(flag), {doc} = \"Alternative.\")]\nstruct RawNames;"
            );
            assert_plain(&source, &["Anchor. Selected.", "Anchor. Alternative."]);
        }
    }
    assert_plain(
        r#"#![r#doc = "Inner."]
#![r#cfg_attr(flag, r#doc = "Selected.")]
struct Inner;"#,
        &["Inner.", "Inner. Selected."],
    );
}

#[test]
fn review_boolean_cfg_literals_preserve_fences_and_cost_no_atom_slots() {
    for (predicate, expected) in [
        ("true", vec!["Anchor. Selected."]),
        ("false", vec!["Anchor."]),
        ("not(false)", vec!["Anchor. Selected."]),
        ("all(true, flag)", vec!["Anchor.", "Anchor. Selected."]),
        ("any(false, not(true))", vec!["Anchor."]),
    ] {
        let source = format!(
            "/// Anchor.\n#[cfg_attr({predicate}, doc = \"Selected.\")]\nstruct Constants;"
        );
        assert_plain(&source, &expected);
    }
    let source = r#"/// Before.
///
#[cfg_attr(all(true, not(false)), doc = "```rust")]
/// let should_not_be_prose = 42;
/// ```
///
/// After.
struct Example;"#;
    assert_plain(source, &["Before. After."]);

    let source = format!(
        "{}struct ManyConstants;",
        "#[cfg_attr(true, doc = \"Word.\")]\n".repeat(12)
    );
    assert_plain(&source, &[&["Word."; 12].join(" ")]);
}

#[test]
fn review_unrelated_edge_attributes_do_not_disable_safe_reflow() {
    const PROSE: &str = "Documentation contains a longer explanation that must wrap into several shorter lines without changing nearby attributes.";
    for attribute in [
        "#[derive(Debug)]",
        "#[cfg_attr(flag, derive(Debug))]",
        "#[allow(dead_code)]",
    ] {
        for leading in [false, true] {
            let docs = format!("/// {PROSE}\n");
            let source = if leading {
                format!("{attribute}\n{docs}struct Example;\n")
            } else {
                format!("{docs}{attribute}\nstruct Example;\n")
            };
            let original = assert_plain(&source, &[PROSE]);
            assert!(original[0].is_reflow_safe(), "source:\n{source}");
            let wrapped = apply_reflow(&source, 40);
            assert_ne!(wrapped, source);
            assert_eq!(wrapped.matches(attribute).count(), 1);
            assert!(wrapped.lines().all(|line| line.chars().count() <= 40));
            syn::parse_file(&wrapped).unwrap();
            assert_plain(&wrapped, &[PROSE]);
            assert_eq!(apply_reflow(&wrapped, 40), wrapped);
        }
    }
}

#[test]
fn review_intervening_attribute_on_a_doc_line_is_not_rewritten() {
    let source = "/// Before.\n#[allow(dead_code)] /// Documentation contains a longer explanation that must not replace the preceding attribute.\nstruct Example;\n";
    let original = assert_plain(source, &["Before. Documentation contains a longer explanation that must not replace the preceding attribute."]);
    assert!(!original[0].is_reflow_safe());
    assert_eq!(apply_reflow(source, 40), source);
}

#[test]
fn review_nonreconstructable_doc_prefixes_never_duplicate_rust_syntax() {
    const PROSE: &str = "A long paragraph with enough words to wrap across several lines and expose any repeated syntax.";
    for source in [
        format!("struct Previous; #[doc = \"{PROSE}\"] struct Example;"),
        format!("#[doc =\n    \"{PROSE}\"]\nstruct Example;"),
        format!("#[\ndoc = \"{PROSE}\"]\nstruct Example;"),
        format!("#[allow(dead_code)] #[doc = \"{PROSE}\"]\nstruct Example;"),
    ] {
        syn::parse_file(&source).unwrap();
        let original = assert_plain(&source, &[PROSE]);
        assert!(!original[0].is_reflow_safe(), "source:\n{source}");
        assert_eq!(apply_reflow(&source, 40), source);
    }
}

#[test]
fn review_unsupported_cfg_syntax_skips_only_its_incomplete_documentation_run() {
    assert_plain(
        r#"/// Before.
#[cfg_attr(version("1.85"), doc = "```rust")]
/// let should_not_be_prose = 42;
/// ```
/// After.
struct Unsupported;
/// Retained.
struct Independent;"#,
        &["Retained."],
    );
}

#[test]
fn review_opaque_doc_macros_cannot_inject_attributes_from_their_arguments() {
    assert_plain(
        r#"/// Anchor.
#[doc = opaque!(doc = "Decoy.", #[cfg_attr(flag, doc = "Another decoy.")])]
/// Tail.
struct Opaque;"#,
        &["Anchor. Tail."],
    );
}

#[test]
fn review_word_subranges_map_through_unicode_and_conditional_source_gaps() {
    let source = r#"/// Préface.
#[cfg_attr(flag, doc = "Café naïf.
Deuxième ligne.")]
#[cfg_attr(not(flag), doc = "日本語 résumé.")]
/// Finale.
struct Words;"#;
    let chunks = chunks(source);
    assert_eq!(chunks.len(), 2);
    for chunk in &chunks {
        for (fragment, _) in chunk.iter().filter(|(range, _)| !range.is_empty()) {
            for start in fragment.clone() {
                for end in start + 1..=fragment.end {
                    let requested = start..end;
                    let mappings = chunk.find_spans(requested.clone());
                    assert_eq!(mappings.len(), 1);
                    let (mapped, span) = mappings.first().unwrap();
                    assert_eq!(mapped, &requested);
                    assert_eq!(
                        load_span_from(source.as_bytes(), *span).unwrap(),
                        sub_chars(chunk.as_str(), requested)
                    );
                }
            }
        }
    }
}

#[test]
fn ordinary_doc_attributes_and_comments_keep_prose_around_code() {
    assert_plain(
        r####"
/// Before the example.
#[doc = r#"
```rust
let erased_code = 123;
```
"#]
#[doc = "After the example."]
struct Ordinary;
"####,
        &["Before the example. After the example."],
    );
}

#[test]
fn alternative_fence_openings_produce_two_examples_chunks() {
    let chunks = assert_plain(
        r####"
/// # Examples
#[cfg_attr(feature = "nightly", doc = "```rust")]
#[cfg_attr(not(feature = "nightly"), doc = "```rust,ignore")]
/// let erased_example = unimplemented!();
/// ```
struct Example;
"####,
        &["Examples", "Examples"],
    );
    let mut raw: Vec<_> = chunks.iter().map(|chunk| chunk.as_str()).collect();
    raw.sort();
    assert_ne!(raw[0], raw[1], "different fence literals must survive");
    assert!(raw.iter().any(|text| text.contains("```rust,ignore")));
    assert!(raw.iter().any(|text| !text.contains("```rust,ignore")));
}

#[test]
fn unix_conditional_no_run_and_ignore_fences_exclude_example_code() {
    let chunks = assert_plain(
        r####"
/// # Example
///
#[cfg_attr(unix, doc = "```no_run")]
#[cfg_attr(not(unix), doc = "```ignore")]
/// use std::path::Path;
/// assert_eq!(Path::new("./is_a_directory/").is_dir(), true);
/// ```
struct Example;
"####,
        &["Example", "Example"],
    );
    assert!(chunks
        .iter()
        .any(|chunk| chunk.as_str().contains("```no_run")));
    assert!(chunks
        .iter()
        .any(|chunk| chunk.as_str().contains("```ignore")));
}

#[test]
fn alternative_code_bodies_do_not_swallow_surrounding_prose() {
    assert_plain(
        r####"
/// Before.
///
/// ```rust
#[cfg_attr(fast, doc = "let erased_fast = 1;")]
#[cfg_attr(not(fast), doc = "let erased_slow = 2;")]
/// ```
///
/// After.
struct Bodies;
"####,
        &["Before. After.", "Before. After."],
    );
}

#[test]
fn negative_branch_prose_is_not_lost() {
    assert_plain(
        r####"
#[doc = "Start."]
#[cfg_attr(feature = "enabled", doc = "Enabled prose.")]
#[cfg_attr(not(feature = "enabled"), doc = "Disabled prose.")]
#[doc = "End."]
struct Negative;
"####,
        &["Start. Enabled prose. End.", "Start. Disabled prose. End."],
    );
}

#[test]
fn independent_atoms_form_all_four_projections() {
    assert_plain(
        r####"
/// Anchor.
#[cfg_attr(alpha, doc = "Alpha.")]
#[cfg_attr(beta, doc = "Beta.")]
struct Independent;
"####,
        &[
            "Anchor.",
            "Anchor. Alpha.",
            "Anchor. Beta.",
            "Anchor. Alpha. Beta.",
        ],
    );
}

#[test]
fn repeated_atoms_and_negation_are_correlated_across_attributes() {
    assert_plain(
        r####"
/// Anchor.
#[cfg_attr(flag, doc = "First yes.")]
#[cfg_attr(not(flag), doc = "No.")]
#[cfg_attr(all(flag, flag), doc = "Second yes.")]
#[cfg_attr(not(not(flag)), doc = "Third yes.")]
struct Correlated;
"####,
        &["Anchor. First yes. Second yes. Third yes.", "Anchor. No."],
    );
    assert_plain(
        r####"
/// Anchor.
#[cfg_attr(any(a, b), doc = "Either.")]
#[cfg_attr(all(a, b), doc = "Both.")]
#[cfg_attr(not(any(a, b)), doc = "Neither.")]
#[cfg_attr(all(a, not(b)), doc = "Only a.")]
#[cfg_attr(all(not(a), b), doc = "Only b.")]
struct CompositeCorrelation;
"####,
        &[
            "Anchor. Neither.",
            "Anchor. Either. Only a.",
            "Anchor. Either. Only b.",
            "Anchor. Either. Both.",
        ],
    );
}

#[test]
fn boolean_predicate_table_deduplicates_selected_identities_not_assignments() {
    let cases = [
        ("all()", vec!["Anchor. Selected."]),
        ("any()", vec!["Anchor."]),
        ("not(all())", vec!["Anchor."]),
        ("not(any())", vec!["Anchor. Selected."]),
        ("all(a, not(a))", vec!["Anchor."]),
        ("any(a, not(a))", vec!["Anchor. Selected."]),
        ("all(a, b)", vec!["Anchor.", "Anchor. Selected."]),
        ("any(a, b, a,)", vec!["Anchor.", "Anchor. Selected."]),
        (
            "not(any(not(a), not(b)))",
            vec!["Anchor.", "Anchor. Selected."],
        ),
        ("all(any(), a)", vec!["Anchor."]),
        ("any(all(), a)", vec!["Anchor. Selected."]),
    ];
    for (predicate, expected) in cases {
        let source =
            format!("/// Anchor.\n#[cfg_attr({predicate}, doc = \"Selected.\")]\nstruct Boolean;");
        assert_plain(&source, &expected);
    }
}

#[test]
fn nested_cfg_attr_conjoins_parents_and_preserves_sibling_order() {
    assert_plain(
        r####"
/// Anchor.
#[cfg_attr(a,
    doc = "Outer.",
    cfg_attr(b, doc = "Both."),
    cfg_attr(not(b), doc = "Only outer."),
    doc = "Tail.",
)]
#[cfg_attr(not(a), cfg_attr(b, doc = "Only inner."))]
struct Nested;
"####,
        &[
            "Anchor.",
            "Anchor. Only inner.",
            "Anchor. Outer. Both. Tail.",
            "Anchor. Outer. Only outer. Tail.",
        ],
    );
}

#[test]
fn nested_contradiction_cannot_select_unreachable_prose() {
    assert_plain(
        r####"
/// Anchor.
#[cfg_attr(a, cfg_attr(not(a), doc = "Unreachable."), doc = "Reachable.")]
struct Contradiction;
"####,
        &["Anchor.", "Anchor. Reachable."],
    );
}

#[test]
fn multiple_doc_attributes_and_trailing_commas_share_one_condition() {
    assert_plain(
        r####"
/// Before.
#[cfg_attr(flag,
    doc = "First.",
    allow(dead_code),
    doc = r#"Second."#,
)]
/// After.
struct Multiple;
"####,
        &["Before. After.", "Before. First. Second. After."],
    );
}

#[test]
fn cfg_atoms_are_not_evaluated_against_the_host() {
    assert_plain(
        r####"
/// Portable.
#[cfg_attr(unix, doc = "Unix.")]
#[cfg_attr(not(unix), doc = "Not unix.")]
#[cfg_attr(feature = "never_enabled_in_this_test", doc = "Feature.")]
struct Portable;
"####,
        &[
            "Portable. Unix.",
            "Portable. Unix. Feature.",
            "Portable. Not unix.",
            "Portable. Not unix. Feature.",
        ],
    );
}

#[test]
fn key_value_atoms_ignore_token_spacing_but_distinguish_values() {
    assert_plain(
        r####"
/// Anchor.
#[cfg_attr(feature="one", doc = "One.")]
#[cfg_attr(not(feature = "one"), doc = "Not one.")]
#[cfg_attr(feature = "two", doc = "Two.")]
struct Features;
"####,
        &[
            "Anchor. One.",
            "Anchor. One. Two.",
            "Anchor. Not one.",
            "Anchor. Not one. Two.",
        ],
    );
}

#[test]
fn raw_and_normal_multiline_literals_build_complete_alternative_fences() {
    assert_plain(
        r####"
#[doc = "Before."]
#[cfg_attr(raw, doc = r##"
```rust
let erased_raw = r#"a raw literal"#;
```
"##)]
#[cfg_attr(not(raw), doc = "
```text
erased normal multiline example
```
")]
#[doc = r#"After."#]
struct Multiline;
"####,
        &["Before. After.", "Before. After."],
    );
}

#[test]
fn inner_and_outer_attributes_on_the_same_lines_are_separate_runs() {
    assert_plain(
        r####"
#![doc = "Inner."]
#![cfg_attr(flag, doc = "Inner option.")]
#[doc = "Outer."]
#[cfg_attr(flag, doc = "Outer option.")]
struct Outer;
"####,
        &[
            "Inner.",
            "Inner. Inner option.",
            "Outer.",
            "Outer. Outer option.",
        ],
    );
}

#[test]
fn neighboring_items_do_not_share_a_run_even_on_one_line() {
    assert_plain(
        r####"
#[doc = "First."] #[cfg_attr(a, doc = "First option.")] struct First;
#[doc = "Second."] #[cfg_attr(a, doc = "Second option.")] struct Second;
"####,
        &[
            "First.",
            "First. First option.",
            "Second.",
            "Second. Second option.",
        ],
    );
}

#[test]
fn nested_scopes_keep_parent_inner_and_child_documentation_independent() {
    assert_plain(
        r####"
/// Parent.
#[cfg_attr(a, doc = "Parent option.")]
mod parent {
    //! Inside.
    #![cfg_attr(a, doc = "Inside option.")]
    /// Child.
    #[cfg_attr(a, doc = "Child option.")]
    fn child() {}
}
/// Sibling.
fn sibling() {}
"####,
        &[
            "Parent.",
            "Parent. Parent option.",
            "Inside.",
            "Inside. Inside option.",
            "Child.",
            "Child. Child option.",
            "Sibling.",
        ],
    );
}

#[test]
fn unrelated_attributes_bridge_a_documentation_run() {
    assert_plain(
        r####"
/// Before.
#[allow(dead_code)]
#[cfg_attr(a, doc = "Optional.")]
#[derive(Clone)]
#[repr(C)]
/// After.
struct Bridged;
"####,
        &["Before. After.", "Before. Optional. After."],
    );
}

#[test]
fn conditions_only_affecting_non_doc_attributes_do_not_multiply_projections() {
    assert_plain(
        r####"
/// Before.
#[cfg_attr(a, allow(dead_code), cfg_attr(b, derive(Clone)),)]
#[cfg_attr(not(a), repr(C))]
/// After.
struct NonDoc;
"####,
        &["Before. After."],
    );
}

#[test]
fn bare_doc_token_groups_and_unrelated_attribute_payloads_are_decoys() {
    assert_plain(
        r####"
const TOKENS: () = (doc = "Expression decoy.");
consume!(doc = "Macro decoy.");
consume!([doc = "Bracket decoy."]);
#[other(doc = "Attribute payload decoy.")]
#[other(cfg_attr(flag, doc = "Nested payload decoy."))]
#[tool::doc = "Qualified doc decoy."]
#[tool::cfg_attr(flag, doc = "Qualified cfg decoy.")]
/// Actual.
#[cfg_attr(flag, other(doc = "Conditional payload decoy."), doc = "Selected.")]
struct Actual;
"####,
        &["Actual.", "Actual. Selected."],
    );
}

#[test]
fn attribute_spellings_inside_strings_and_ordinary_comments_are_not_docs() {
    assert_plain(
        r####"
const DECOY: &str = r###"#[doc = "String decoy."]
#[cfg_attr(a, doc = "Another string decoy.")]"###;
// #[doc = "Line comment decoy."]
/* #[cfg_attr(a, doc = "Block comment decoy.")] */
fn no_docs() {}
"####,
        &[],
    );
}

#[test]
fn equal_branch_text_at_distinct_source_locations_keeps_two_projections() {
    let source = r####"
/// Before.
#[cfg_attr(flag, doc = "Identical prose.")]
#[cfg_attr(not(flag), doc = "Identical prose.")]
/// After.
struct Identities;
"####;
    let chunks = assert_plain(
        source,
        &[
            "Before. Identical prose. After.",
            "Before. Identical prose. After.",
        ],
    );
    let mappings: Vec<_> = chunks
        .iter()
        .map(|chunk| chunk.iter().collect::<Vec<_>>())
        .collect();
    assert_ne!(
        mappings[0], mappings[1],
        "literal identity includes source location"
    );
}

#[test]
fn repeated_literal_text_within_one_projection_is_not_deduplicated() {
    assert_plain(
        r####"
/// Anchor.
#[cfg_attr(a, doc = "Echo.", doc = "Echo.")]
#[cfg_attr(not(a), doc = "Echo.")]
struct Repeated;
"####,
        &["Anchor. Echo. Echo.", "Anchor. Echo."],
    );
}

#[test]
fn exactly_ten_independent_atoms_produce_all_literal_subsets() {
    let mut source = String::from("/// Anchor.\n");
    for atom in 0..10 {
        source.push_str(&format!(
            "#[cfg_attr(atom_{atom}, doc = \"Word{atom}.\")]\n"
        ));
    }
    source.push_str("struct AtCap;");
    let mut expected = Vec::new();
    for assignment in 0..(1_usize << 10) {
        let mut text = String::from("Anchor.");
        for atom in 0..10 {
            if assignment & (1 << atom) != 0 {
                text.push_str(&format!(" Word{atom}."));
            }
        }
        expected.push(text);
    }
    let expected: Vec<_> = expected.iter().map(String::as_str).collect();
    assert_plain(&source, &expected);
}

#[test]
fn over_cap_skips_the_whole_run_but_not_other_items_or_repeated_atoms() {
    let mut source = String::from("/// Skipped unconditional prose.\n");
    for atom in 0..11 {
        source.push_str(&format!(
            "#[cfg_attr(atom_{atom}, doc = \"Skipped{atom}.\")]\n"
        ));
    }
    source.push_str("struct OverCap;\n");
    // A fresh run gets a fresh budget, and eleven uses of one atom cost only one.
    source.push_str("/// Retained.\n#[cfg_attr(all(");
    source.push_str(&["repeated"; 11].join(", "));
    source.push_str("), doc = \"Repeated.\")]\nstruct RepeatedAtom;\n");
    for atom in 0..11 {
        source.push_str(&format!(
            "#[cfg_attr(independent_{atom}, doc = \"Separate{atom}.\")] struct Separate{atom};\n"
        ));
    }
    let mut expected = vec![
        String::from("Retained."),
        String::from("Retained. Repeated."),
    ];
    // The unselected projection on a run containing no unconditional docs is empty
    // and must not become a checkable chunk.
    for atom in 0..11 {
        expected.push(format!("Separate{atom}."));
    }
    let expected: Vec<_> = expected.iter().map(String::as_str).collect();
    assert_plain(&source, &expected);
}

#[test]
fn unicode_raw_delimiters_and_multiline_spans_roundtrip_in_every_projection() {
    let source = r####"
/// Préface 🦀.
#[cfg_attr(été, doc = r###"Café naïf 🐋.
Deuxième ligne."###)]
#[cfg_attr(not(été), doc = "Αλφάβητο 日本語.
Autre ligne.")]
#[doc = r##"
```rust
let erased_unicode = "🦀";
```
"##]
#[doc = r#"Fin résumé."#]
struct Unicode;
"####;
    assert_plain(
        source,
        &[
            "Préface 🦀. Café naïf 🐋. Deuxième ligne. Fin résumé.",
            "Préface 🦀. Αλφάβητο 日本語. Autre ligne. Fin résumé.",
        ],
    );
}

#[test]
fn empty_doc_literals_keep_their_newline_separators() {
    let chunks = assert_plain(
        r#"#[doc = ""]
#[doc = "First."]
#[doc = ""]
#[doc = "Second."]
#[doc = ""]
struct EmptyFragments;"#,
        &["First. Second."],
    );
    assert_eq!(chunks[0].as_str(), "\nFirst.\n\nSecond.\n");
}

#[test]
fn raw_identifier_atoms_and_raw_string_values_are_correlated() {
    assert_plain(
        r####"
/// Anchor.
#[cfg_attr(r#unix, doc = "Unix.")]
#[cfg_attr(not(unix), doc = "Not unix.")]
#[cfg_attr(r#feature = r#"one"#, doc = "One.")]
#[cfg_attr(not(feature = "one"), doc = "Not one.")]
struct RawIdentifiers;
"####,
        &[
            "Anchor. Unix. One.",
            "Anchor. Unix. Not one.",
            "Anchor. Not unix. One.",
            "Anchor. Not unix. Not one.",
        ],
    );
}

fn apply_reflow(source: &str, max_line_length: usize) -> String {
    use crate::checker::Checker;
    let chunks = chunks(source);
    let reflow =
        crate::reflow::Reflow::new(crate::config::ReflowConfig { max_line_length }).unwrap();
    let suggestions = reflow
        .check(&ContentOrigin::TestEntityRust, &chunks)
        .unwrap();
    let patches = suggestions.iter().map(|suggestion| {
        crate::Patch::from(crate::BandAid::from((
            suggestion.replacements[0].clone(),
            &suggestion.span,
        )))
    });
    let mut output = Vec::new();
    crate::action::apply_patches(patches, source, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn long_conditional_doc_attributes_stay_intact_while_another_item_wraps() {
    const PROSE: &str = "This single line contains enough explanation to require wrapping into several shorter documentation attributes.";
    let cases = [
        (
            format!(
                "#[cfg_attr(r#unix, doc = \"{PROSE}\")]\n\
                 #[cfg_attr(not(unix), doc = \"Alternative. {PROSE}\")]"
            ),
            2,
        ),
        (
            format!(
                "#[cfg_attr(feature = \"outer\", doc = \"{PROSE}\",\n\
                 cfg_attr(feature = \"inner\", doc = r###\"Nested. {PROSE}\"###), allow(dead_code))]"
            ),
            3,
        ),
        (
            format!(
                "#[cfg_attr(any(a, b), doc = r##\"{PROSE}\"##)]\n\
                 #[cfg_attr(all(not(a), not(b)), doc = \"Alternative. {PROSE}\")]"
            ),
            2,
        ),
    ];
    for (attributes, versions) in cases {
        let conditional = format!("/// Anchor.\n{attributes}\n/// Tail.\nstruct Conditional;\n");
        assert!(conditional.lines().any(|line| line.chars().count() > 40));
        let original = chunks(&conditional);
        assert_eq!(original.len(), versions);
        assert!(original.iter().all(|chunk| !chunk.is_reflow_safe()));
        assert_span_roundtrip(&conditional, &original);
        assert_eq!(apply_reflow(&conditional, 40), conditional);

        // A skipped conditional run must not prevent an independent long doc
        // comment on another item from being wrapped normally.
        let ordinary = format!("/// {PROSE}\nstruct Ordinary;\n");
        let wrapped_ordinary = apply_reflow(&ordinary, 40);
        assert_ne!(wrapped_ordinary, ordinary);
        assert_eq!(
            plain_texts(&chunks(&wrapped_ordinary)),
            plain_texts(&chunks(&ordinary))
        );
        let source = format!("{conditional}\n{ordinary}");
        let before = plain_texts(&chunks(&source));
        let wrapped = apply_reflow(&source, 40);
        assert_eq!(wrapped, format!("{conditional}\n{wrapped_ordinary}"));
        syn::parse_file(&wrapped).expect("all conditional wrappers must remain valid Rust");
        let after = chunks(&wrapped);
        assert_eq!(plain_texts(&after), before);
        assert_span_roundtrip(&wrapped, &after);
        assert_eq!(apply_reflow(&wrapped, 40), wrapped);
    }
}

#[test]
fn crlf_comments_and_conditional_string_tokens_preserve_their_boundaries() {
    let source = r####"
/// Before.
#[doc = "Ordinary."]
#[cfg_attr(flag, doc = r##"Raw.
Second line."##)]
#[cfg_attr(not(flag), doc = "Normal.
Second line.")]
/// After.
struct WindowsLineEndings;
"####
        .replace('\n', "\r\n");
    assert_plain(
        &source,
        &[
            "Before. Ordinary. Raw. Second line. After.",
            "Before. Ordinary. Normal. Second line. After.",
        ],
    );
}

#[test]
fn mixed_multiline_comment_formats_cannot_be_reflowed_as_one_format() {
    use crate::checker::Checker;
    let chunks = assert_plain(
        r####"
#[doc = "A paragraph with enough words that it will need wrapping."]
/** More words
and another line. */
struct Mixed;
"####,
        &["A paragraph with enough words that it will need wrapping. More words and another line."],
    );
    assert!(!chunks[0].is_reflow_safe());
    let reflow = crate::reflow::Reflow::new(Default::default()).unwrap();
    assert!(reflow
        .check(&ContentOrigin::TestEntityRust, &chunks)
        .unwrap()
        .is_empty());
}

#[test]
fn fence_styles_lengths_and_info_strings_can_be_mixed_conditionally() {
    for (first, second, close) in [
        ("```no_run", "```ignore", "```"),
        ("```rust,edition2021", "```rust,compile_fail", "```"),
        ("~~~rust", "~~~text", "~~~"),
        ("````rust", "````ignore", "````"),
        ("```", "```ignore", "```"),
        (" ```no_run", " ```ignore", " ```"),
    ] {
        let source = format!(
            "/// Before.\n///\n\
             #[cfg_attr(any(a, b), doc = \"{first}\")]\n\
             #[cfg_attr(all(not(a), not(b)), doc = \"{second}\")]\n\
             /// let erased_code = unimplemented!();\n/// {close}\n///\n/// After.\nstruct Fences;"
        );
        assert_plain(&source, &["Before. After.", "Before. After."]);
    }
}

#[test]
fn several_nested_attributes_on_one_line_keep_literal_order() {
    assert_plain(
        r####"#[doc = "Start."] #[cfg_attr(a, doc = "One.", cfg_attr(not(b), doc = "Two."))] #[doc = "End."] struct Inline;"####,
        &["Start. End.", "Start. One. End.", "Start. One. Two. End."],
    );
}

#[test]
fn opaque_documentation_macros_inside_conditions_keep_fences_continuous() {
    assert_plain(
        r####"
/// Before.
///
/// ```rust
#[cfg_attr(flag, doc = include_str!("not-expanded.rs"))]
/// let erased_code = 1;
/// ```
///
/// After.
struct Opaque;
"####,
        &["Before. After.", "Before. After."],
    );
}

#[cfg(feature = "hunspell")]
#[test]
fn spelling_checks_both_branches_and_reports_shared_source_only_once() {
    use crate::checker::{Checker, Checkers};
    let source = r####"
/// Mispellllled
#[cfg_attr(flag, doc = "```rust")]
#[cfg_attr(not(flag), doc = "```ignore")]
/// let ignored_mispellllled = 0;
/// ```
///
/// Mispellllled
#[cfg_attr(flag, doc = "Mispellllled")]
#[cfg_attr(not(flag), doc = "Mispellllled")]
struct Spelling;
"####;
    let chunks = chunks(source);
    assert_eq!(chunks.len(), 2);
    let checker = Checkers::new(crate::Config {
        nlprules: None,
        zet: None,
        spellbook: None,
        ..Default::default()
    })
    .unwrap();
    let suggestions = checker
        .check(&ContentOrigin::TestEntityRust, &chunks)
        .unwrap();
    // Two shared literals and one literal in each conditional branch.
    assert_eq!(suggestions.len(), 4);
    let mut spans = std::collections::HashSet::new();
    for suggestion in suggestions {
        assert_eq!(
            load_span_from(source.as_bytes(), suggestion.span).unwrap(),
            "Mispellllled"
        );
        assert!(spans.insert(suggestion.span));
    }
}

#[test]
fn reflow_skips_structurally_gapped_documentation() {
    const PROSE: &str =
        "Documentation contains enough ordinary words to require wrapping across several lines.";
    for gap in ["\n", "#[allow(dead_code)]\n", "// Not documentation.\n"] {
        let source = format!("/// Before.\n{gap}/// {PROSE}\nstruct Gapped;\n");
        let original = assert_plain(&source, &[&format!("Before. {PROSE}")]);
        assert_eq!(original.len(), 1);
        assert!(!original[0].is_reflow_safe());
        assert_eq!(apply_reflow(&source, 40), source);
    }
}

#[test]
fn documented_examples_extract_with_roundtripping_source_spans() {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let guide = include_str!("../docs/conditional-documentation.md");
    let mut sources = Vec::new();
    let mut source = None;
    for event in Parser::new(guide) {
        match event {
            Event::Start(Tag::CodeBlock(_)) => source = Some(String::new()),
            Event::Text(text) => {
                if let Some(source) = &mut source {
                    source.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) => sources.push(source.take().unwrap()),
            _ => {}
        }
    }
    assert_eq!(sources.len(), 3);
    for (source, count) in sources.iter().zip([2, 3, 1]) {
        let chunks = chunks(source);
        assert_eq!(chunks.len(), count, "example:\n{source}");
        assert_span_roundtrip(source, &chunks);
    }
}
