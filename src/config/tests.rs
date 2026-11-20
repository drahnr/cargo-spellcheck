use super::*;
use assert_matches::assert_matches;

#[test]
fn can_serialize_to_toml() {
    let config = dbg!(Config::full());
    assert_matches!(config.to_toml(), Ok(_s));
}

#[test]
fn project_config_works() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".config")
        .join("spellcheck.toml");
    assert_matches!(Config::load_from(&path), Ok(_));
}

#[test]
fn all() {
    let _ = Config::parse(
        r#"
dev_comments = true
skip-readme = true

[Hunspell]
lang = "en_US"
search_dirs = ["/usr/lib64/hunspell"]
extra_dictionaries = ["/home/bernhard/test.dic"]
			"#,
    )
    .unwrap();
}

#[test]
fn empty() {
    assert!(Config::parse(
        r#"
			"#,
    )
    .is_ok());
}
#[test]
fn partial_1() {
    let _cfg = Config::parse(
        r#"
[hunspell]
lang = "en_GB"
search_dirs = ["/usr/lib64/hunspell"]
extra_dictionaries = ["/home/bernhard/test.dic"]
			"#,
    )
    .unwrap();
}

#[test]
fn partial_3() {
    let cfg = Config::parse(
        r#"
[Hunspell]
lang = "de_AT"
search_dirs = ["/usr/lib64/hunspell"]
extra_dictionaries = ["/home/bernhard/test.dic"]
			"#,
    )
    .unwrap();
    let _hunspell = cfg.hunspell.expect("Must contain hunspell cfg");
}

#[test]
fn partial_4() {
    let cfg = Config::parse(
        r#"
[Hunspell]
lang = "en_US"
			"#,
    )
    .unwrap();
    let _hunspell = cfg.hunspell.expect("Must contain hunspell cfg");
}

#[test]
fn partial_5() {
    assert!(Config::parse(
        r#"
[hUNspell]
lang = "en_US"
			"#,
    )
    .is_err());
}

#[test]
fn partial_6() {
    let cfg = Config::parse(
        r#"
[hunspell]
			"#,
    )
    .unwrap();
    let _hunspell = cfg.hunspell.expect("Must contain hunspell cfg");
}

#[test]
fn partial_7() {
    let cfg = Config::parse(
        r#"
[Hunspell.quirks]
allow_concatenation = true
allow_dashes = true
transform_regex = ["^'([^\\s])'$", "^[0-9]+x$"]
			"#,
    )
    .unwrap();
    let _hunspell = cfg.hunspell.expect("Must contain hunspell cfg");
}

#[test]
fn partial_8() {
    let cfg = Config::parse(
        r#"
[Hunspell]
search_dirs = ["/search/1", "/search/2"]
skip_os_lookups = true
			"#,
    )
    .unwrap();

    let hunspell: HunspellConfig = cfg.hunspell.expect("Must contain hunspell cfg");
    assert!(hunspell.skip_os_lookups);

    let search_dirs = hunspell.search_dirs;
    let search_dirs2: Vec<_> = search_dirs.as_ref().clone();
    assert!(!search_dirs2.is_empty());

    assert_eq!(search_dirs.iter(false).count(), 2);

    #[cfg(target_os = "linux")]
    assert_eq!(search_dirs.iter(true).count(), 5);

    #[cfg(target_os = "windows")]
    assert_eq!(search_dirs.iter(true).count(), 2);

    #[cfg(target_os = "macos")]
    assert!(search_dirs.iter(true).count() >= 3);
}

#[test]
fn partial_9() {
    let cfg = Config::parse(
        r#"
[Reflow]
max_line_length = 42
"#,
    )
    .unwrap();
    assert_eq!(
        cfg.reflow.expect("Must contain reflow cfg").max_line_length,
        42
    );
}
