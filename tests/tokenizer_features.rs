//! Ensure tokenization artifacts and grammar configuration follow separate features.

#[test]
fn hunspell_retains_its_grammar_feature_dependency() {
    assert!(!cfg!(feature = "hunspell") || cfg!(feature = "nlprules"));
}

#[test]
fn grammar_configuration_follows_the_grammar_feature() {
    assert_eq!(
        cargo_spellcheck::Config::default().nlprules.is_some(),
        cfg!(feature = "nlprules")
    );
}

#[test]
fn tokenization_and_grammar_artifacts_follow_enabled_features() {
    let output = std::path::Path::new(env!("OUT_DIR"));
    assert!(output.join("en_tokenizer.bin").is_file());
    assert_eq!(
        output.join("en_rules.bin").is_file(),
        cfg!(feature = "nlprules")
    );
}
