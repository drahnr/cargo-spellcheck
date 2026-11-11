//! Executable examples of structural, host-independent documentation projections.

use super::{CheckableChunk, ContentOrigin, Documentation};
use crate::documentation::{CfgBudget, Clusters};
use crate::util::{load_span_from, sub_chars};

fn chunks(source: &str) -> Vec<CheckableChunk> {
    let docs = Documentation::load_from_str(ContentOrigin::TestEntityRust, source, true, false);
    docs.get(&ContentOrigin::TestEntityRust)
        .unwrap_or(&[])
        .to_vec()
}

fn chunks_with_budget(source: &str, budget: CfgBudget) -> Vec<CheckableChunk> {
    Clusters::load_from_str_with_budget(source, true, false, budget)
        .expect("valid Rust documentation")
        .into()
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

fn witness_values(chunk: &CheckableChunk, atoms: &[&str]) -> Vec<bool> {
    let example = chunk
        .cfg_example()
        .expect("a conditional projection has a witness");
    let terms: Vec<_> = example
        .strip_prefix("all(")
        .and_then(|example| example.strip_suffix(')'))
        .unwrap_or(example)
        .split(", ")
        .collect();
    assert_eq!(terms.len(), atoms.len(), "incomplete witness: {example}");
    atoms
        .iter()
        .map(|atom| {
            let positive = terms.contains(atom);
            let negative = terms.contains(&format!("not({atom})").as_str());
            assert_ne!(
                positive, negative,
                "missing or conflicting {atom}: {example}"
            );
            positive
        })
        .collect()
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
    /// Prose long enough to trigger reflow next to unrelated attributes.
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
    /// Prose that would expose duplicated Rust syntax if an unsafe prefix were repeated.
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

/// Regression test for <https://github.com/drahnr/cargo-spellcheck/issues/263>
#[test]
fn documented_platform_dependent_code_fences_regression_263() {
    let chunks = assert_plain(
        r###"
/// # Examples
///
#[cfg_attr(unix, doc = "```no_run")]
#[cfg_attr(not(unix), doc = "```ignore")]
/// use std::path::Path;
/// assert!(Path::new("./is_a_directory/").is_dir());
/// ```
///
/// More explanation after the example.
struct Directory;
"###,
        &[
            "Examples More explanation after the example.",
            "Examples More explanation after the example.",
        ],
    );
    let mut opening_fences = Vec::new();
    let host_example = if cfg!(unix) { "unix" } else { "not(unix)" };
    for chunk in &chunks {
        assert_eq!(chunk.as_str().matches("```").count(), 2);
        assert!(chunk.as_str().contains("use std::path::Path;"));
        assert!(chunk
            .as_str()
            .contains("assert!(Path::new(\"./is_a_directory/\").is_dir());"));
        // Both symbolic projections are checked. Only select the host-matching
        // witness for the platform-specific assertion below.
        if chunk.cfg_example() != Some(host_example) {
            continue;
        }
        opening_fences.push(
            chunk
                .as_str()
                .lines()
                .find(|line| line.starts_with("```"))
                .unwrap(),
        );
    }
    opening_fences.sort();
    #[cfg(unix)]
    assert_eq!(opening_fences, ["```no_run"]);
    #[cfg(not(unix))]
    assert_eq!(opening_fences, ["```ignore"]);
}

#[test]
fn directly_attached_cfg_constrains_docs_before_or_after_it() {
    for source in [
        "#[cfg(foo)]\n#[doc = \"xxx\"]\nstruct X;",
        "#[doc = \"xxx\"]\n#[cfg(foo)]\nstruct X;",
        "#[r#cfg(r#foo)]\n#[r#doc = \"xxx\"]\nstruct X;",
    ] {
        let chunks = assert_plain(source, &["xxx"]);
        assert_eq!(chunks[0].cfg_example(), Some("foo"));
    }
    let chunks = assert_plain("/// Ordinary.\nstruct Ordinary;", &["Ordinary."]);
    assert_eq!(chunks[0].cfg_example(), None);
}

#[test]
fn incompatible_item_cfg_guards_have_no_documentation_projection() {
    for guards in ["#[cfg(false)]", "#[cfg(foo)]\n#[cfg(not(foo))]"] {
        let source = format!(
            "{guards}\n#[doc = \"Unreachable.\"]\nstruct Removed;\n/// Retained.\nstruct Other;"
        );
        let chunks = assert_plain(&source, &["Retained."]);
        assert_eq!(chunks[0].cfg_example(), None);
    }
}

#[test]
fn cfg_generated_by_cfg_attr_is_a_conditional_item_constraint() {
    let chunks = assert_plain(
        r#"/// Anchor.
#[cfg_attr(a, cfg(b), doc = "Selected.")]
#[cfg_attr(not(a), doc = "Alternative.")]
struct Conditional;"#,
        &["Anchor. Selected.", "Anchor. Alternative."],
    );
    for chunk in chunks {
        let values = witness_values(&chunk, &["a", "b"]);
        let (a, b) = (values[0], values[1]);
        assert!(!a || b, "the generated cfg requires a to imply b");
        assert_eq!(chunk.as_str().contains("Selected."), a);
        assert_eq!(chunk.as_str().contains("Alternative."), !a);
    }
    let chunks = assert_plain(
        "/// Anchor.\n#[cfg_attr(a, cfg(not(a)), doc = \"Unreachable.\")]\nstruct X;",
        &["Anchor."],
    );
    assert_eq!(chunks[0].cfg_example(), Some("not(a)"));
}

#[test]
fn multiple_values_of_one_cfg_key_can_be_enabled_together() {
    struct Case {
        key: &'static str,
        first_value: &'static str,
        second_value: &'static str,
    }
    let cases = [
        Case {
            key: "feature",
            first_value: "std",
            second_value: "serde",
        },
        Case {
            key: "target_feature",
            first_value: "sse2",
            second_value: "avx",
        },
        Case {
            key: "target_family",
            first_value: "unix",
            second_value: "wasm",
        },
        Case {
            key: "custom_option",
            first_value: "a",
            second_value: "b",
        },
    ];
    let expected_texts = ["Both."];
    for Case {
        key,
        first_value,
        second_value,
    } in cases
    {
        let source = format!(
            r#"#[cfg(all({key} = "{first_value}", {key} = "{second_value}"))]
#[doc = "Both."]
struct MultipleValues;"#
        );
        let chunks = assert_plain(&source, &expected_texts);
        let expected = format!(r#"all({key} = "{first_value}", {key} = "{second_value}")"#);
        assert_eq!(chunks[0].cfg_example(), Some(expected.as_str()));
    }
}

#[test]
fn conditional_projections_keep_one_satisfying_example_each() {
    let projections = assert_plain(
        "/// Anchor.\n#[cfg_attr(any(a, b), doc = \"Either.\")]\n#[cfg_attr(all(a, b), doc = \"Both.\")]\nstruct X;",
        &["Anchor.", "Anchor. Either.", "Anchor. Either. Both."],
    );
    for chunk in projections {
        let values = witness_values(&chunk, &["a", "b"]);
        let (a, b) = (values[0], values[1]);
        assert_eq!(chunk.as_str().contains("Both."), a && b);
        assert_eq!(chunk.as_str().contains("Either."), a || b);
        assert_eq!(chunk.clone().cfg_example(), chunk.cfg_example());
    }
    for predicate in ["all()", "any()"] {
        let source =
            format!("/// Anchor.\n#[cfg_attr({predicate}, doc = \"Optional.\")]\nstruct X;");
        assert_eq!(chunks(&source)[0].cfg_example(), Some("all()"));
    }
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
    // Cases pair a cfg predicate with the expected distinct documentation texts.
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
fn variant_cap_accepts_all_four_literal_subsets() {
    let source = r#"/// Release handbook.
#[cfg_attr(introduction, doc = "Introduction.")]
#[cfg_attr(installation, doc = "Installation.")]
struct AtCap;"#;
    let budget = CfgBudget {
        variants: 4,
        samples: 4,
        ..Default::default()
    };
    let expected = [
        "Release handbook.",
        "Release handbook. Installation.",
        "Release handbook. Introduction.",
        "Release handbook. Introduction. Installation.",
    ];
    let projections = chunks_with_budget(source, budget);
    let actual = plain_texts(&projections);
    assert_eq!(actual, expected);
    assert_span_roundtrip(source, &projections);
}

#[test]
fn over_variant_cap_retains_complete_projections_and_satisfying_constraints() {
    let source = r#"/// Handbook.
#[cfg_attr(introduction, doc = "Introduction.")]
#[cfg_attr(installation, doc = "Installation.")]
#[cfg(any(introduction, installation))]
#[cfg_attr(introduction, cfg(not(blocked)), doc = "Verified.")]
#[cfg_attr(not(introduction), doc = "Offline.")]
struct OverCap;"#;
    let atoms = ["introduction", "installation", "blocked"];
    let budget = CfgBudget {
        variants: 2,
        samples: 2,
        ..Default::default()
    };
    // Cases map [introduction, installation, blocked] to the expected complete prose.
    let expected_cases = [
        ([false, true, false], "Handbook. Installation. Offline."),
        ([false, true, true], "Handbook. Installation. Offline."),
        ([true, false, false], "Handbook. Introduction. Verified."),
        (
            [true, true, false],
            "Handbook. Introduction. Installation. Verified.",
        ),
    ];
    // Three valid selections exceed the two-variant budget.
    let expected_max_projections = budget.samples;
    let projections = chunks_with_budget(source, budget);
    assert_span_roundtrip(source, &projections);
    assert!(!projections.is_empty());
    assert!(projections.len() <= expected_max_projections);
    for chunk in projections {
        let values = witness_values(&chunk, &atoms);
        let expected = expected_cases
            .iter()
            .find(|(input, _)| input.as_slice() == values)
            .map(|(_, expected)| *expected)
            .expect("the witness must satisfy the item constraints");
        let actual = plain_texts(std::slice::from_ref(&chunk));
        assert_eq!(actual, [expected]);
    }
}

#[test]
fn over_variant_cap_does_not_limit_neighboring_items_or_repeated_atoms() {
    let source = r#"/// Release overview.
#[cfg_attr(planning, doc = "Plan the release.")]
#[cfg_attr(packaging, doc = "Package the release.")]
#[cfg_attr(publishing, doc = "Publish the release.")]
struct OverCap;
/// Retained.
#[cfg_attr(all(repeated, repeated, repeated), doc = "Repeated.")]
struct RepeatedAtom;
#[cfg_attr(offline, doc = "Offline guide.")] struct OfflineGuide;
#[cfg_attr(online, doc = "Online guide.")] struct OnlineGuide;"#;
    let budget = CfgBudget {
        variants: 4,
        samples: 4,
        ..Default::default()
    };
    let expected_other_texts = [
        "Offline guide.",
        "Online guide.",
        "Retained.",
        "Retained. Repeated.",
    ];
    let projections = chunks_with_budget(source, budget);
    assert_span_roundtrip(source, &projections);
    let other: Vec<_> = projections
        .into_iter()
        .filter(|chunk| !chunk.as_str().contains("Release overview."))
        .collect();
    let actual_other_texts = plain_texts(&other);
    assert_eq!(actual_other_texts, expected_other_texts);
}

#[test]
fn over_variant_cap_keeps_late_conditional_fences_complete_and_erases_code() {
    let source = r#"/// Operators recieve release guidance.
#[cfg_attr(topic_0, doc = "Prepare the workspace.")]
#[cfg_attr(topic_1, doc = "Download the release.")]
#[cfg_attr(topic_2, doc = "Verify the signature.")]
///
#[cfg_attr(executable, doc = "```rust")]
#[cfg_attr(not(executable), doc = "```text")]
/// let status = "seperated";
/// ```
///
/// Record the deployment outcome.
struct ReleaseProcedure;"#;
    // Each topic pairs its enabling cfg flag with the corresponding prose fragment.
    let topics = [
        ("topic_0", "Prepare the workspace."),
        ("topic_1", "Download the release."),
        ("topic_2", "Verify the signature."),
    ];
    let atoms: Vec<_> = topics
        .iter()
        .map(|(atom, _)| *atom)
        .chain(std::iter::once("executable"))
        .collect();
    let budget = CfgBudget {
        variants: 4,
        samples: 4,
        ..Default::default()
    };
    let expected_max_projections = budget.samples;
    let expected_prose_before = "Operators recieve release guidance.";
    let expected_prose_after = "Record the deployment outcome.";
    let expected_code_line = r#"let status = "seperated";"#;
    let expected_code_typo = "seperated";
    let expected_closer = "```";
    let expected_fence_count = 2;
    let projections = chunks_with_budget(source, budget);
    assert!(!projections.is_empty());
    assert!(projections.len() <= expected_max_projections);
    assert_span_roundtrip(source, &projections);
    let mut selections = std::collections::HashSet::new();
    for chunk in projections {
        let values = witness_values(&chunk, &atoms);
        let expected_opener = if values[3] { "```rust" } else { "```text" };
        let expected_fence_lines = [expected_opener, expected_code_line, expected_closer];
        let actual_lines: Vec<_> = chunk.as_str().lines().map(str::trim).collect();
        assert!(actual_lines
            .windows(expected_fence_lines.len())
            .any(|lines| lines == expected_fence_lines));
        let actual_fence_count = chunk.as_str().matches(expected_closer).count();
        assert_eq!(actual_fence_count, expected_fence_count);
        let mut expected_parts = vec![expected_prose_before];
        for ((_, expected_topic), selected) in topics.iter().zip(&values) {
            if *selected {
                expected_parts.push(*expected_topic);
            }
        }
        expected_parts.push(expected_prose_after);
        let expected_texts = vec![expected_parts.join(" ")];
        let actual_texts = plain_texts(std::slice::from_ref(&chunk));
        assert_eq!(actual_texts, expected_texts);
        let erased = chunk.erase_cmark(&Default::default());
        assert!(!erased.as_str().contains(expected_code_typo));
        assert!(
            selections.insert(chunk.as_str().to_owned()),
            "duplicate projection"
        );
    }
}

#[test]
fn feature_conjunction_counts_literal_selections_not_assignments() {
    let source = r#"/// Storage engine.
#[cfg_attr(all(feature = "cache", feature = "compression", feature = "encryption"),
    doc = "Full storage support.")]
struct StorageEngine;"#;
    let atoms = [
        r#"feature = "cache""#,
        r#"feature = "compression""#,
        r#"feature = "encryption""#,
    ];
    let budget = CfgBudget {
        variants: 2,
        samples: 2,
        ..Default::default()
    };
    let expected = ["Storage engine.", "Storage engine. Full storage support."];
    let projections = chunks_with_budget(source, budget);
    let actual = plain_texts(&projections);
    assert_eq!(actual, expected);
    assert_span_roundtrip(source, &projections);
    for chunk in projections {
        let values = witness_values(&chunk, &atoms);
        let expected_full_support = values.iter().all(|value| *value);
        let actual_full_support = chunk.as_str().contains("Full storage support.");
        assert_eq!(actual_full_support, expected_full_support);
    }
}

#[test]
fn tautologies_fit_one_variant_and_keep_a_complete_witness() {
    let source = r#"/// File operations.
#[cfg_attr(all(any(read, not(read)), any(write, not(write)), any(sync, not(sync))),
    doc = "Always documented.")]
struct FileOperations;"#;
    let atoms = ["read", "write", "sync"];
    let budget = CfgBudget {
        variants: 1,
        samples: 1,
        ..Default::default()
    };
    let expected = ["File operations. Always documented."];
    let projections = chunks_with_budget(source, budget);
    let actual = plain_texts(&projections);
    assert_eq!(actual, expected);
    assert_span_roundtrip(source, &projections);
    witness_values(&projections[0], &atoms);
}

#[test]
fn contradictory_nested_branch_fits_one_variant() {
    let source = r#"/// Connection remains documented.
#[cfg_attr(online, cfg_attr(all(not(online), authenticated, encrypted),
    doc = "Unreachable connection details."))]
struct Connection;"#;
    let atoms = ["online", "authenticated", "encrypted"];
    let budget = CfgBudget {
        variants: 1,
        samples: 1,
        ..Default::default()
    };
    let expected = ["Connection remains documented."];
    let projections = chunks_with_budget(source, budget);
    let actual = plain_texts(&projections);
    assert_eq!(actual, expected);
    assert_span_roundtrip(source, &projections);
    witness_values(&projections[0], &atoms);
}

#[test]
fn direct_cfg_reduces_independent_doc_guards_to_two_variants() {
    let source = r#"#[cfg_attr(parse, doc = "Parsed.")]
#[cfg(all(parse, resolve))]
#[cfg_attr(resolve, doc = "Resolved.")]
#[cfg_attr(emit, doc = "Emitted.")]
struct CompilationPipeline;"#;
    let atoms = ["parse", "resolve", "emit"];
    let budget = CfgBudget {
        variants: 2,
        samples: 2,
        ..Default::default()
    };
    let expected = ["Parsed. Resolved.", "Parsed. Resolved. Emitted."];
    let projections = chunks_with_budget(source, budget);
    let actual = plain_texts(&projections);
    assert_eq!(actual, expected);
    assert_span_roundtrip(source, &projections);
    for chunk in projections {
        let values = witness_values(&chunk, &atoms);
        let expected_constraints_hold = values[0] && values[1];
        assert!(expected_constraints_hold);
        let expected_emitted = values[2];
        let actual_emitted = chunk.as_str().contains("Emitted.");
        assert_eq!(actual_emitted, expected_emitted);
    }
}

#[test]
fn nested_cfg_implications_constrain_the_whole_documentation_run() {
    let projections = assert_plain(
        r#"/// Deployment.
#[cfg(any(manual, automatic))]
#[cfg_attr(automatic, cfg(ready), doc = "Automatic deployment.")]
#[cfg_attr(manual,
    doc = "Manual deployment.",
    cfg_attr(not(automatic), cfg(override_ready), doc = "Operator override."),
)]
#[cfg_attr(all(manual, automatic),
    cfg(not(override_ready)), doc = "Combined deployment.",
)]
struct Deployment;"#,
        &[
            "Deployment. Automatic deployment.",
            "Deployment. Manual deployment. Operator override.",
            "Deployment. Automatic deployment. Manual deployment. Combined deployment.",
        ],
    );
    for chunk in projections {
        let values = witness_values(&chunk, &["manual", "automatic", "ready", "override_ready"]);
        let (manual, automatic, ready, override_ready) =
            (values[0], values[1], values[2], values[3]);
        assert!(manual || automatic);
        assert!(!automatic || ready);
        assert!(!(manual && !automatic) || override_ready);
        assert!(!(manual && automatic) || !override_ready);
        assert_eq!(chunk.as_str().contains("Automatic deployment."), automatic);
        assert_eq!(chunk.as_str().contains("Manual deployment."), manual);
        assert_eq!(
            chunk.as_str().contains("Operator override."),
            manual && !automatic,
        );
        assert_eq!(
            chunk.as_str().contains("Combined deployment."),
            manual && automatic,
        );
    }
}

#[test]
fn variant_cap_accepts_four_selections_including_the_empty_selection() {
    let source = r#"#[cfg_attr(introduction, doc = "Introduction.")]
#[cfg_attr(installation, doc = "Installation.")]
struct Handbook;"#;
    let budget = CfgBudget {
        variants: 4,
        samples: 4,
        ..Default::default()
    };
    let expected = [
        "Installation.",
        "Introduction.",
        "Introduction. Installation.",
    ];
    let projections = chunks_with_budget(source, budget);
    let actual = plain_texts(&projections);
    assert_eq!(actual, expected);
    assert_span_roundtrip(source, &projections);
}

#[test]
fn variant_cap_counts_empty_selection_before_discarding_empty_chunks() {
    // Five valid selections include the empty selection; sampling may omit any one.
    let source = r#"#[cfg(any(not(appendix), all(not(overview), not(requirements))))]
#[cfg_attr(appendix, doc = "Appendix.")]
#[cfg_attr(overview, doc = "Overview.")]
#[cfg_attr(requirements, doc = "Requirements.")]
struct OverVariantCap;
/// Independent release notes.
struct ReleaseNotes;"#;
    let atoms = ["appendix", "overview", "requirements"];
    let sections = ["Appendix.", "Overview.", "Requirements."];
    let budget = CfgBudget {
        variants: 4,
        samples: 4,
        ..Default::default()
    };
    let expected_max_projections = budget.samples;
    let expected_other_texts = ["Independent release notes."];
    let projections = chunks_with_budget(source, budget);
    assert_span_roundtrip(source, &projections);
    let (other, retained): (Vec<_>, Vec<_>) = projections
        .into_iter()
        .partition(|chunk| chunk.as_str().contains("Independent release notes."));
    let actual_other_texts = plain_texts(&other);
    assert_eq!(actual_other_texts, expected_other_texts);
    assert!(!retained.is_empty());
    assert!(retained.len() <= expected_max_projections);
    let mut selections = std::collections::HashSet::new();
    for chunk in retained {
        let values = witness_values(&chunk, &atoms);
        let expected_constraint_holds = !values[0] || (!values[1] && !values[2]);
        assert!(expected_constraint_holds);
        let expected_text = sections
            .iter()
            .zip(&values)
            .filter_map(|(section, selected)| selected.then_some(*section))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(!expected_text.is_empty(), "empty selections are not chunks");
        let expected_texts = vec![expected_text.clone()];
        let actual_texts = plain_texts(std::slice::from_ref(&chunk));
        assert_eq!(actual_texts, expected_texts);
        assert!(selections.insert(expected_text), "duplicate projection");
    }
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

fn assert_reflows(source: &str, max_line_length: usize) -> String {
    let wrapped = apply_reflow(source, max_line_length);
    assert_ne!(wrapped, source, "the long attribute must actually wrap");
    assert!(wrapped.matches("#[doc = ").count() > 1);
    assert!(
        wrapped
            .lines()
            .all(|line| line.chars().count() <= max_line_length),
        "wrapped source:\n{wrapped}"
    );
    syn::parse_file(&wrapped).expect("wrapped attributes must remain valid Rust");
    let after = chunks(&wrapped);
    assert_eq!(plain_texts(&after), plain_texts(&chunks(source)));
    assert_span_roundtrip(&wrapped, &after);
    assert_eq!(
        apply_reflow(&wrapped, max_line_length),
        wrapped,
        "reflow must be stable"
    );
    wrapped
}

#[test]
fn long_single_line_doc_attributes_wrap_without_changing_their_contents() {
    /// A single prose line exceeding the test's reflow width.
    const PROSE: &str =
        "One line which is quite long and needs to be wrapped into several shorter lines.";
    for (open, close) in [
        ("\"", "\""),
        ("r\"", "\""),
        ("r#\"", "\"#"),
        ("r###\"", "\"###"),
    ] {
        let source = format!("#[doc = {open}{PROSE}{close}]\nstruct Long;\n");
        assert!(source.lines().next().unwrap().chars().count() > 40);
        let original = assert_plain(&source, &[PROSE]);
        assert!(original[0].is_reflow_safe());

        assert_reflows(&source, 40);
    }
}

#[test]
fn long_single_line_doc_attributes_wrap_with_neighboring_docs_and_indentation() {
    /// Prose used to verify wrapping preserves neighboring docs and indentation.
    const PROSE: &str = "This single line contains enough explanation to require wrapping while preserving its neighboring documentation.";
    for (open, close) in [("\"", "\""), ("r##\"", "\"##")] {
        for nested in [false, true] {
            let indent = if nested { "    " } else { "" };
            let item = format!(
                "{indent}#[doc = {open}Before.{close}]\n\
                 {indent}#[doc = {open}{PROSE}{close}]\n\
                 {indent}#[doc = {open}After.{close}]\n\
                 {indent}struct Long;\n"
            );
            let source = if nested {
                format!("mod nested {{\n{item}}}\n")
            } else {
                item
            };
            let expected = format!("Before. {PROSE} After.");
            let original = assert_plain(&source, &[&expected]);
            assert!(original[0].is_reflow_safe());

            assert_reflows(&source, 40);
        }
    }
}

#[test]
fn long_conditional_doc_attributes_stay_intact_while_another_item_wraps() {
    /// Shared long prose contrasting conditional attributes with an ordinary reflowable item.
    const PROSE: &str = "This single line contains enough explanation to require wrapping into several shorter documentation attributes.";
    // Cases pair conditional attributes with their expected documentation variant count.
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
        // attribute on another item from being wrapped normally.
        let ordinary = format!("#[doc = \"{PROSE}\"]\nstruct Ordinary;\n");
        let wrapped_ordinary = assert_reflows(&ordinary, 40);
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
    struct Fences {
        selected_opener: &'static str,
        alternate_opener: &'static str,
        closer: &'static str,
    }
    let cases = [
        Fences {
            selected_opener: "```no_run",
            alternate_opener: "```ignore",
            closer: "```",
        },
        Fences {
            selected_opener: "```rust,edition2021",
            alternate_opener: "```rust,compile_fail",
            closer: "```",
        },
        Fences {
            selected_opener: "~~~rust",
            alternate_opener: "~~~text",
            closer: "~~~",
        },
        Fences {
            selected_opener: "````rust",
            alternate_opener: "````ignore",
            closer: "````",
        },
        Fences {
            selected_opener: "```",
            alternate_opener: "```ignore",
            closer: "```",
        },
        Fences {
            selected_opener: " ```no_run",
            alternate_opener: " ```ignore",
            closer: " ```",
        },
    ];
    let expected = ["Before. After.", "Before. After."];
    for Fences {
        selected_opener,
        alternate_opener,
        closer,
    } in cases
    {
        let source = format!(
            r#"/// Before.
///
#[cfg_attr(any(a, b), doc = "{selected_opener}")]
#[cfg_attr(all(not(a), not(b)), doc = "{alternate_opener}")]
/// let erased_code = unimplemented!();
/// {closer}
///
/// After.
struct FencedExample;"#
        );
        assert_plain(&source, &expected);
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
