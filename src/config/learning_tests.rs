use super::{Config, HunspellConfig};
use fs_err as fs;
use std::path::PathBuf;

type Backend = fn(&mut Config) -> &mut Option<HunspellConfig>;

const BACKENDS: &[(&str, Backend)] = &[
    #[cfg(feature = "hunspell")]
    ("Hunspell", |config| &mut config.hunspell),
    #[cfg(feature = "zet")]
    ("ZSpell", |config| &mut config.zet),
    #[cfg(feature = "spellbook")]
    ("Spellbook", |config| &mut config.spellbook),
];

fn disabled_config() -> Config {
    Config {
        dev_comments: false,
        skip_readme: false,
        hunspell: None,
        zet: None,
        spellbook: None,
        nlprules: None,
        reflow: None,
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "cargo-spellcheck-shared-learning-config-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&path).unwrap();
        let dir = Self(fs::canonicalize(path).unwrap());
        fs::create_dir(dir.path("nested")).unwrap();
        for name in [
            "learning.dic",
            "first.dic",
            "second.dic",
            "other.dic",
            "disabled.dic",
        ] {
            fs::write(dir.path(name), "0\n").unwrap();
        }
        dir
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn spelling_config(&self, target: Option<&str>, extras: &[&str]) -> HunspellConfig {
        HunspellConfig {
            learning_dictionary: target.map(|path| self.path(path)),
            extra_dictionaries: extras.iter().map(|path| self.path(path)).collect(),
            search_dirs: vec![self.0.clone()].into(),
            skip_os_lookups: true,
            ..HunspellConfig::default()
        }
    }

    fn all_backends(&self, target: Option<&str>) -> Config {
        let mut config = disabled_config();
        for &(_, backend) in BACKENDS {
            *backend(&mut config) = Some(self.spelling_config(target, &["learning.dic"]));
        }
        config
    }

    fn load(&self, config: &Config) -> Config {
        let path = self.path("spellcheck.toml");
        fs::write(&path, config.to_toml().unwrap()).unwrap();
        Config::load_from(path).unwrap().unwrap()
    }

    fn expected_target(&self) -> Option<PathBuf> {
        (!BACKENDS.is_empty()).then(|| self.path("learning.dic"))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn each_compiled_backend_can_select_the_target_on_its_own() {
    let dir = TempDir::new();
    for &(name, backend) in BACKENDS {
        let mut config = disabled_config();
        *backend(&mut config) = Some(dir.spelling_config(Some("learning.dic"), &["learning.dic"]));
        assert_eq!(
            config.learning_dictionary().unwrap(),
            Some(dir.path("learning.dic")),
            "{name} must select its target without another spelling engine"
        );
    }
}

#[test]
fn does_not_fall_back_to_the_first_extra_dictionary() {
    let dir = TempDir::new();
    let mut config = disabled_config();
    for &(_, backend) in BACKENDS {
        *backend(&mut config) = Some(dir.spelling_config(None, &["first.dic", "second.dic"]));
        assert_eq!(config.learning_dictionary().unwrap(), None);
    }
}

#[test]
fn disabling_a_backend_removes_its_target() {
    let dir = TempDir::new();
    for &(_, backend) in BACKENDS {
        let mut config = disabled_config();
        *backend(&mut config) = Some(dir.spelling_config(Some("disabled.dic"), &["disabled.dic"]));
        assert!(config.learning_dictionary().unwrap().is_some());
        *backend(&mut config) = None;
        assert_eq!(config.learning_dictionary().unwrap(), None);
    }
}

#[test]
fn disabled_backend_does_not_conflict_with_an_enabled_target() {
    let dir = TempDir::new();
    for (index, &(_, backend)) in BACKENDS.iter().enumerate() {
        for &(_, other) in &BACKENDS[index + 1..] {
            let mut config = disabled_config();
            *backend(&mut config) =
                Some(dir.spelling_config(Some("learning.dic"), &["learning.dic"]));
            *other(&mut config) =
                Some(dir.spelling_config(Some("disabled.dic"), &["disabled.dic"]));
            *other(&mut config) = None;
            assert_eq!(
                config.learning_dictionary().unwrap(),
                Some(dir.path("learning.dic"))
            );
        }
    }
}

#[test]
fn rejects_distinct_targets_on_any_pair_of_enabled_backends() {
    let dir = TempDir::new();
    for (index, &(name, backend)) in BACKENDS.iter().enumerate() {
        for &(other_name, other) in &BACKENDS[index + 1..] {
            let mut config = disabled_config();
            let extras = &["first.dic", "second.dic"];
            *backend(&mut config) = Some(dir.spelling_config(Some("first.dic"), extras));
            *other(&mut config) = Some(dir.spelling_config(Some("second.dic"), extras));
            let error = config.learning_dictionary().unwrap_err().to_string();
            assert!(
                error.contains("Conflicting learning_dictionary targets"),
                "{name} and {other_name}: {error}"
            );
        }
    }
}

#[test]
fn target_must_be_in_the_selecting_backends_extra_dictionaries() {
    let dir = TempDir::new();
    for &(name, backend) in BACKENDS {
        let mut config = disabled_config();
        *backend(&mut config) = Some(dir.spelling_config(Some("learning.dic"), &["other.dic"]));
        let error = config.learning_dictionary().unwrap_err().to_string();
        assert!(error.contains("extra_dictionaries"), "{name}: {error}");
        assert!(error.contains(name), "{name}: {error}");
    }
}

#[test]
fn target_must_be_loaded_by_every_other_enabled_backend() {
    let dir = TempDir::new();
    for (index, &(_, backend)) in BACKENDS.iter().enumerate() {
        for (other_index, &(other_name, other)) in BACKENDS.iter().enumerate() {
            if index == other_index {
                continue;
            }
            let mut config = disabled_config();
            *backend(&mut config) =
                Some(dir.spelling_config(Some("learning.dic"), &["learning.dic"]));
            *other(&mut config) = Some(dir.spelling_config(None, &["other.dic"]));
            let error = config.learning_dictionary().unwrap_err().to_string();
            assert!(error.contains("extra_dictionaries"), "{error}");
            assert!(error.contains(other_name), "{error}");
        }
    }
}

#[test]
fn rejects_mixed_languages_even_for_session_only_learning() {
    let dir = TempDir::new();
    for (index, &(name, backend)) in BACKENDS.iter().enumerate() {
        for &(other_name, other) in &BACKENDS[index + 1..] {
            for target in [None, Some("learning.dic")] {
                let mut config = disabled_config();
                *backend(&mut config) = Some(dir.spelling_config(target, &["learning.dic"]));
                let mut other_config = dir.spelling_config(target, &["learning.dic"]);
                other_config.lang = toml::from_str::<HunspellConfig>("lang = 'de_DE'")
                    .unwrap()
                    .lang;
                *other(&mut config) = Some(other_config);
                let error = config.learning_dictionary().unwrap_err().to_string();
                assert!(
                    error.contains("same language"),
                    "{name} and {other_name}, target {target:?}: {error}"
                );
            }
        }
    }
}

#[test]
fn any_enabled_backend_can_select_a_target_loaded_by_all_backends() {
    let dir = TempDir::new();
    for &(_, selecting_backend) in BACKENDS {
        let mut config = dir.all_backends(None);
        selecting_backend(&mut config)
            .as_mut()
            .unwrap()
            .learning_dictionary = Some(dir.path("learning.dic"));
        assert_eq!(
            config.learning_dictionary().unwrap(),
            Some(dir.path("learning.dic"))
        );
    }
}

#[test]
fn all_enabled_backends_can_explicitly_share_the_same_target() {
    let dir = TempDir::new();
    assert_eq!(
        dir.all_backends(Some("learning.dic"))
            .learning_dictionary()
            .unwrap(),
        dir.expected_target()
    );
}

#[test]
fn no_enabled_spelling_backend_means_session_only_learning() {
    assert_eq!(disabled_config().learning_dictionary().unwrap(), None);
}

#[test]
fn active_learning_targets_must_be_regular_files_when_present() {
    let dir = TempDir::new();
    for &(name, backend) in BACKENDS {
        let mut config = disabled_config();
        *backend(&mut config) = Some(dir.spelling_config(Some("nested"), &["learning.dic"]));
        let error = config.learning_dictionary().unwrap_err().to_string();
        assert!(error.contains("learning_dictionary"), "{error}");
        assert!(error.contains("writable regular file"), "{error}");
        assert!(error.contains(name), "{error}");
    }
}

#[test]
fn active_learning_registrations_must_be_existing_files() {
    let dir = TempDir::new();
    for &(name, backend) in BACKENDS {
        for registration in ["missing.dic", "nested"] {
            let mut config = disabled_config();
            *backend(&mut config) =
                Some(dir.spelling_config(Some("learning.dic"), &["learning.dic", registration]));
            let error = config.learning_dictionary().unwrap_err().to_string();
            assert!(error.contains("extra_dictionaries"), "{error}");
            assert!(error.contains("existing dictionary file"), "{error}");
            assert!(error.contains(name), "{error}");
        }
    }
}

#[test]
fn invalid_other_registrations_are_not_hidden_by_a_registered_target() {
    let dir = TempDir::new();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("missing.dic", dir.path("broken.dic")).unwrap();
        std::os::unix::fs::symlink("loop.dic", dir.path("loop.dic")).unwrap();
    }
    let invalid = [
        "learning.dic/child.dic",
        #[cfg(unix)]
        "broken.dic",
        #[cfg(unix)]
        "loop.dic",
    ];
    for &(name, backend) in BACKENDS {
        for target in ["learning.dic", "missing.dic"] {
            for registration in &invalid {
                let mut config = disabled_config();
                *backend(&mut config) =
                    Some(dir.spelling_config(Some(target), &[target, registration]));
                let error = config.learning_dictionary().unwrap_err().to_string();
                assert!(error.contains("extra_dictionaries"), "{name}: {error}");
                assert!(error.contains(registration), "{name}: {error}");
                let before = config.to_toml().unwrap();
                if target == "missing.dic" {
                    assert!(config
                        .omit_missing_learning_dictionary(&dir.path(target))
                        .is_err());
                    assert_eq!(config.to_toml().unwrap(), before);
                }
            }
        }
    }
    assert!(!dir.path("missing.dic").exists());
}

#[test]
fn cli_filter_excludes_invalid_learning_settings_and_preserves_reflow() {
    use super::args::Args;

    let dir = TempDir::new();
    for &(name, selected) in BACKENDS {
        let invalid = dir.spelling_config(Some("nested"), &["nested"]);
        let mut config = Config {
            hunspell: Some(invalid.clone()),
            zet: Some(invalid.clone()),
            spellbook: Some(invalid),
            ..Config::default()
        };
        *selected(&mut config) = Some(dir.spelling_config(Some("learning.dic"), &["learning.dic"]));
        let mut loaded = dir.load(&config);
        let reflow_width = loaded.reflow.as_ref().unwrap().max_line_length;
        Args::checker_selection_override(Some(&[name.parse().unwrap()]), &mut loaded).unwrap();
        assert_eq!(
            loaded.learning_dictionary().unwrap(),
            Some(dir.path("learning.dic"))
        );
        assert_eq!(
            loaded.reflow.as_ref().unwrap().max_line_length,
            reflow_width
        );
    }
}

#[test]
fn load_from_accepts_equivalent_relative_targets_across_backends() {
    let dir = TempDir::new();
    let mut config = disabled_config();
    for (index, &(_, backend)) in BACKENDS.iter().enumerate() {
        let target = if index % 2 == 0 {
            "./learning.dic"
        } else {
            "nested/../learning.dic"
        };
        let mut spelling = dir.spelling_config(None, &[]);
        spelling.learning_dictionary = Some(PathBuf::from(target));
        spelling.extra_dictionaries = vec![PathBuf::from("nested/../learning.dic")];
        spelling.search_dirs = vec![PathBuf::from(".")].into();
        *backend(&mut config) = Some(spelling);
    }
    let mut loaded = dir.load(&config);
    assert_eq!(loaded.learning_dictionary().unwrap(), dir.expected_target());
    for &(_, backend) in BACKENDS {
        let spelling = backend(&mut loaded).as_ref().unwrap();
        assert_eq!(spelling.extra_dictionaries, vec![dir.path("learning.dic")]);
        assert!(spelling.learning_dictionary.as_ref().unwrap().is_absolute());
    }
}

#[test]
fn load_from_accepts_absolute_parent_components_in_targets_and_registrations() {
    let dir = TempDir::new();
    let mut config = dir.all_backends(Some("learning.dic"));
    for (index, &(_, backend)) in BACKENDS.iter().enumerate() {
        let spelling = backend(&mut config).as_mut().unwrap();
        spelling.extra_dictionaries = vec![dir.path("nested/../learning.dic")];
        if index % 2 == 0 {
            spelling.learning_dictionary = Some(dir.path("nested/../learning.dic"));
        }
    }
    let loaded = dir.load(&config);
    assert_eq!(loaded.learning_dictionary().unwrap(), dir.expected_target());
}

#[cfg(unix)]
#[test]
fn load_from_accepts_absolute_symlinks_in_targets_and_registrations() {
    let dir = TempDir::new();
    std::os::unix::fs::symlink(dir.path("learning.dic"), dir.path("alias.dic")).unwrap();
    let mut config = dir.all_backends(Some("learning.dic"));
    for (index, &(_, backend)) in BACKENDS.iter().enumerate() {
        let spelling = backend(&mut config).as_mut().unwrap();
        spelling.extra_dictionaries = vec![dir.path("alias.dic")];
        if index % 2 == 0 {
            spelling.learning_dictionary = Some(dir.path("alias.dic"));
        }
    }
    let loaded = dir.load(&config);
    assert_eq!(loaded.learning_dictionary().unwrap(), dir.expected_target());
}

#[test]
fn load_from_defers_invalid_learning_targets_until_after_backend_filtering() {
    let dir = TempDir::new();
    for &(name, backend) in BACKENDS {
        for target in ["missing.dic", "nested", "other.dic"] {
            let mut config = dir.all_backends(Some("learning.dic"));
            *backend(&mut config) = Some(dir.spelling_config(Some(target), &["learning.dic"]));
            let mut loaded = dir.load(&config);
            assert!(
                loaded.learning_dictionary().is_err(),
                "active {name}: {target}"
            );
            *backend(&mut loaded) = None;
            let expected = (BACKENDS.len() > 1).then(|| dir.path("learning.dic"));
            assert_eq!(
                loaded.learning_dictionary().unwrap(),
                expected,
                "filtered {name}"
            );
        }
    }
    assert!(!dir.path("missing.dic").exists());
}

#[test]
fn load_from_defers_invalid_registered_learning_targets_until_backend_filtering() {
    let dir = TempDir::new();
    #[cfg(unix)]
    {
        use nix::sys::stat::Mode;
        use std::os::unix::fs::symlink;

        nix::unistd::mkfifo(&dir.path("fifo.dic"), Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
        symlink("missing.dic", dir.path("broken.dic")).unwrap();
        symlink("missing-parent", dir.path("broken-parent")).unwrap();
        symlink("loop.dic", dir.path("loop.dic")).unwrap();
    }
    let targets = [
        "nested",
        #[cfg(unix)]
        "fifo.dic",
        #[cfg(unix)]
        "broken.dic",
        #[cfg(unix)]
        "broken-parent/learning.dic",
        #[cfg(unix)]
        "loop.dic",
        "learning.dic/child.dic",
    ];
    for &(name, backend) in BACKENDS {
        for target in targets {
            for relative_target in [false, true] {
                for relative_extra in [false, true] {
                    let mut config = dir.all_backends(Some("learning.dic"));
                    let mut spelling = dir.spelling_config(Some(target), &[target]);
                    if relative_target {
                        spelling.learning_dictionary = Some(PathBuf::from(target));
                    }
                    if relative_extra {
                        spelling.extra_dictionaries = vec![PathBuf::from(target)];
                    }
                    *backend(&mut config) = Some(spelling);
                    let mut loaded = dir.load(&config);
                    assert!(
                        backend(&mut loaded).as_ref().unwrap().extra_dictionaries[0].is_absolute()
                    );
                    assert!(
                        loaded.learning_dictionary().is_err(),
                        "active {name}: {target}"
                    );
                    *backend(&mut loaded) = None;
                    assert_eq!(
                        loaded.learning_dictionary().unwrap(),
                        (BACKENDS.len() > 1).then(|| dir.path("learning.dic")),
                        "filtered {name}: {target}"
                    );

                    // The same invalid registration is strict when it is not a declared target.
                    backend(&mut config).as_mut().unwrap().learning_dictionary = None;
                    let path = dir.path("spellcheck.toml");
                    fs::write(&path, config.to_toml().unwrap()).unwrap();
                    let error = Config::load_from(path).unwrap_err().to_string();
                    assert!(
                        error.contains(&dir.path(target).display().to_string()),
                        "{name}: {error}"
                    );
                    if target == "nested" || target == "fifo.dic" {
                        assert!(error.contains("expected a regular file"), "{name}: {error}");
                    }
                }
            }
        }
    }
    assert!(!dir.path("missing.dic").exists());
}

#[test]
fn load_from_still_rejects_missing_ordinary_extra_dictionaries() {
    let dir = TempDir::new();
    for &(_, backend) in BACKENDS {
        let mut config = dir.all_backends(None);
        let spelling = backend(&mut config).as_mut().unwrap();
        spelling.extra_dictionaries = vec![PathBuf::from("missing.dic")];
        let path = dir.path("spellcheck.toml");
        fs::write(&path, config.to_toml().unwrap()).unwrap();
        let error = Config::load_from(path).unwrap_err().to_string();
        assert!(error.contains("Could not find extra dictionary"), "{error}");
    }
}

#[cfg(not(all(feature = "hunspell", feature = "zet", feature = "spellbook")))]
#[test]
fn load_from_ignores_invalid_learning_targets_and_languages_of_uncompiled_backends() {
    let dir = TempDir::new();
    let uncompiled = &[
        #[cfg(not(feature = "hunspell"))]
        "Hunspell",
        #[cfg(not(feature = "zet"))]
        "ZSpell",
        #[cfg(not(feature = "spellbook"))]
        "Spellbook",
    ];
    for name in uncompiled {
        let mut config = Config::parse(format!(
            "[{name}]\nlang = 'de_DE'\nlearning_dictionary = 'missing.dic'\n"
        ))
        .unwrap();
        for &(_, backend) in BACKENDS {
            *backend(&mut config) =
                Some(dir.spelling_config(Some("learning.dic"), &["learning.dic"]));
        }
        let loaded = dir.load(&config);
        assert_eq!(
            loaded.learning_dictionary().unwrap(),
            dir.expected_target(),
            "{name}"
        );
    }
    assert!(!dir.path("missing.dic").exists());
}

#[cfg(not(any(feature = "hunspell", feature = "zet", feature = "spellbook")))]
#[test]
fn no_compiled_spelling_backend_means_no_learning_target() {
    assert_eq!(Config::default().learning_dictionary().unwrap(), None);
}

#[test]
fn missing_explicit_target_is_resolved_without_creation_and_only_omitted_in_memory() {
    let dir = TempDir::new();
    for &(_, selecting) in BACKENDS {
        let mut config = dir.all_backends(None);
        for &(_, backend) in BACKENDS {
            let spelling = backend(&mut config).as_mut().unwrap();
            spelling.extra_dictionaries = vec![
                PathBuf::from("nested/../absent/learning.dic"),
                dir.path("other.dic"),
            ];
            spelling.search_dirs = vec![].into();
        }
        selecting(&mut config).as_mut().unwrap().learning_dictionary =
            Some(PathBuf::from("absent/learning.dic"));
        let mut loaded = dir.load(&config);
        let target = dir.path("absent/learning.dic");
        assert_eq!(loaded.learning_dictionary().unwrap(), Some(target.clone()));
        let toml = fs::read_to_string(dir.path("spellcheck.toml")).unwrap();
        loaded.omit_missing_learning_dictionary(&target).unwrap();
        for &(_, backend) in BACKENDS {
            assert_eq!(
                backend(&mut loaded).as_ref().unwrap().extra_dictionaries,
                vec![dir.path("other.dic")]
            );
        }
        assert_eq!(
            selecting(&mut loaded).as_ref().unwrap().learning_dictionary,
            Some(target)
        );
        assert_eq!(
            fs::read_to_string(dir.path("spellcheck.toml")).unwrap(),
            toml
        );
        assert!(!dir.path("absent").exists());
    }
}

#[test]
fn missing_relative_explicit_target_resolves_to_an_absolute_path_without_creation() {
    let relative = PathBuf::from(format!(
        "cargo-spellcheck-missing-{}.dic",
        uuid::Uuid::new_v4()
    ));
    let expected = std::env::current_dir()
        .unwrap()
        .canonicalize()
        .unwrap()
        .join(&relative);
    for &(_, backend) in BACKENDS {
        let mut config = disabled_config();
        *backend(&mut config) = Some(HunspellConfig {
            learning_dictionary: Some(relative.clone()),
            extra_dictionaries: vec![relative.clone()],
            ..HunspellConfig::default()
        });
        assert_eq!(
            config.learning_dictionary().unwrap(),
            Some(expected.clone())
        );
        config.omit_missing_learning_dictionary(&expected).unwrap();
        assert!(backend(&mut config)
            .as_ref()
            .unwrap()
            .extra_dictionaries
            .is_empty());
    }
    assert!(!expected.exists());
}

#[test]
fn missing_explicit_targets_still_require_registration_and_a_shared_target() {
    let dir = TempDir::new();
    for &(_, backend) in BACKENDS {
        let mut config = disabled_config();
        *backend(&mut config) = Some(dir.spelling_config(Some("missing.dic"), &["other.dic"]));
        assert!(config
            .learning_dictionary()
            .unwrap_err()
            .to_string()
            .contains("extra_dictionaries"));
        for &(_, other) in BACKENDS {
            let mut config = dir.all_backends(None);
            *backend(&mut config) =
                Some(dir.spelling_config(Some("missing.dic"), &["missing.dic"]));
            if std::ptr::fn_addr_eq(backend, other) {
                continue;
            }
            assert!(config
                .learning_dictionary()
                .unwrap_err()
                .to_string()
                .contains("extra_dictionaries"));
            *other(&mut config) = Some(dir.spelling_config(
                Some("another-missing.dic"),
                &["missing.dic", "another-missing.dic"],
            ));
            assert!(config
                .learning_dictionary()
                .unwrap_err()
                .to_string()
                .contains("Conflicting learning_dictionary"));
            let spelling = other(&mut config).as_mut().unwrap();
            spelling.learning_dictionary = None;
            spelling.lang = toml::from_str::<HunspellConfig>("lang = 'de_DE'")
                .unwrap()
                .lang;
            assert!(config
                .learning_dictionary()
                .unwrap_err()
                .to_string()
                .contains("same language"));
        }
    }
}

#[test]
fn omitting_an_existing_learning_dictionary_leaves_config_and_contents_intact() {
    let dir = TempDir::new();
    let mut config = dir.all_backends(Some("learning.dic"));
    let before = config.to_toml().unwrap();
    config
        .omit_missing_learning_dictionary(&dir.path("learning.dic"))
        .unwrap();
    assert_eq!(config.to_toml().unwrap(), before);
    assert_eq!(fs::read_to_string(dir.path("learning.dic")).unwrap(), "0\n");
}

#[cfg(unix)]
#[test]
fn readonly_explicit_target_is_rejected_only_for_learning_without_modifying_it() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new();
    let path = dir.path("learning.dic");
    fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
    for &(name, backend) in BACKENDS {
        let mut config = disabled_config();
        *backend(&mut config) = Some(dir.spelling_config(Some("learning.dic"), &["learning.dic"]));
        let mut loaded = dir.load(&config);
        let error = loaded.learning_dictionary().unwrap_err().to_string();
        assert!(error.contains("writable regular file"), "{name}: {error}");
        let before = loaded.to_toml().unwrap();
        assert_eq!(
            loaded.include_default_learning_dictionary(&dir.0).unwrap(),
            None
        );
        loaded.omit_missing_learning_dictionary(&path).unwrap();
        assert_eq!(loaded.to_toml().unwrap(), before);
        assert_eq!(fs::read_to_string(&path).unwrap(), "0\n");
    }
    fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[cfg(unix)]
#[test]
fn learning_rejects_fifos_broken_symlinks_and_filesystem_errors_without_opening_them() {
    use nix::sys::stat::Mode;
    use std::os::unix::fs::symlink;

    let dir = TempDir::new();
    nix::unistd::mkfifo(&dir.path("fifo.dic"), Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
    symlink("missing.dic", dir.path("broken.dic")).unwrap();
    symlink("missing-parent", dir.path("broken-parent")).unwrap();
    symlink("loop.dic", dir.path("loop.dic")).unwrap();
    for &(_, backend) in BACKENDS {
        for target in [
            "fifo.dic",
            "broken.dic",
            "broken-parent/learning.dic",
            "loop.dic",
            "learning.dic/child.dic",
        ] {
            assert!(
                Config::validate_learning_dictionary(&dir.path(target)).is_err(),
                "{target}"
            );
            let mut config = disabled_config();
            *backend(&mut config) = Some(dir.spelling_config(Some(target), &[target]));
            assert!(config.learning_dictionary().is_err(), "{target}");
        }
    }
    let mut config = dir.all_backends(None);
    for target in [
        "broken.dic",
        "broken-parent/learning.dic",
        "loop.dic",
        "learning.dic/child.dic",
    ] {
        let before = config.to_toml().unwrap();
        assert!(
            config
                .omit_missing_learning_dictionary(&dir.path(target))
                .is_err(),
            "{target}"
        );
        assert_eq!(config.to_toml().unwrap(), before);
    }
}

#[cfg(unix)]
#[test]
fn missing_explicit_target_canonicalizes_existing_symlink_ancestors() {
    let dir = TempDir::new();
    std::os::unix::fs::symlink(dir.path("nested"), dir.path("alias")).unwrap();
    for &(_, backend) in BACKENDS {
        let mut config = dir.all_backends(None);
        for &(_, other) in BACKENDS {
            other(&mut config).as_mut().unwrap().extra_dictionaries =
                vec![dir.path("nested/missing/learning.dic")];
        }
        backend(&mut config).as_mut().unwrap().learning_dictionary =
            Some(dir.path("alias/missing/learning.dic"));
        let mut loaded = dir.load(&config);
        let target = dir.path("nested/missing/learning.dic");
        assert_eq!(loaded.learning_dictionary().unwrap(), Some(target.clone()));
        assert_eq!(
            backend(&mut loaded).as_ref().unwrap().extra_dictionaries,
            vec![target.clone()]
        );
        loaded.omit_missing_learning_dictionary(&target).unwrap();
        assert!(backend(&mut loaded)
            .as_ref()
            .unwrap()
            .extra_dictionaries
            .is_empty());
        assert!(!dir.path("nested/missing").exists());
    }
}

const PREFERRED: &str = ".config/spellcheck/learnings.dic";
const LEGACY: &str = ".config/learnings.dic";

fn write_default(dir: &TempDir, location: &str) -> PathBuf {
    let path = dir.path(location);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "1\nZzyzxqvocab\n").unwrap();
    path
}

#[test]
fn default_selection_prefers_preferred_then_legacy_without_writing() {
    for (preferred, legacy, selected) in [
        (true, false, PREFERRED),
        (false, true, LEGACY),
        (true, true, PREFERRED),
        (false, false, PREFERRED),
    ] {
        let dir = TempDir::new();
        for (present, location) in [(preferred, PREFERRED), (legacy, LEGACY)] {
            if present {
                write_default(&dir, location);
            }
        }
        let path = Config::default_learning_dictionary(&dir.0).unwrap();
        assert_eq!(path, dir.path(selected));
        assert!(path.is_absolute());
        assert_eq!(
            Config::validate_learning_dictionary(&path).unwrap(),
            (path.clone(), !preferred && !legacy)
        );
        assert_eq!(dir.path(PREFERRED).exists(), preferred);
        assert_eq!(dir.path(LEGACY).exists(), legacy);
        assert_eq!(dir.path(".config").exists(), preferred || legacy);
        assert_eq!(dir.path(".config/spellcheck").exists(), preferred);
        for (present, location) in [(preferred, PREFERRED), (legacy, LEGACY)] {
            if present {
                assert_eq!(
                    fs::read_to_string(dir.path(location)).unwrap(),
                    "1\nZzyzxqvocab\n"
                );
            }
        }
    }
}

#[test]
fn missing_default_under_nonexistent_base_returns_absolute_path_without_creation() {
    let dir = TempDir::new();
    let base = dir.path("missing-project");
    assert_eq!(
        Config::default_learning_dictionary(&base).unwrap(),
        base.join(PREFERRED)
    );
    assert_eq!(
        Config::validate_learning_dictionary(&base.join(PREFERRED)).unwrap(),
        (base.join(PREFERRED), true)
    );
    assert!(!base.exists());
}

#[cfg(unix)]
#[test]
fn validating_missing_defaults_rejects_dangling_ancestors() {
    for ancestor in [".config", ".config/spellcheck"] {
        let dir = TempDir::new();
        let link = dir.path(ancestor);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink("missing-directory", &link).unwrap();
        let target = Config::default_learning_dictionary(&dir.0).unwrap();
        assert_eq!(target, dir.path(PREFERRED));
        assert!(Config::validate_learning_dictionary(&target).is_err());
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(!dir.path("missing-directory").exists());
    }
}

#[cfg(unix)]
#[test]
fn validating_missing_defaults_canonicalizes_existing_ancestors_without_creation() {
    let dir = TempDir::new();
    let alias = dir.path("alias");
    std::os::unix::fs::symlink(dir.path("nested"), &alias).unwrap();
    let target = Config::default_learning_dictionary(&alias).unwrap();
    assert_eq!(
        Config::validate_learning_dictionary(&target).unwrap(),
        (dir.path("nested").join(PREFERRED), true)
    );
    assert!(!dir.path("nested/.config").exists());
}

#[test]
fn dictionary_validation_rechecks_paths_after_prior_success() {
    let dir = TempDir::new();
    let preferred = write_default(&dir, PREFERRED);
    assert_eq!(
        Config::default_learning_dictionary(&dir.0).unwrap(),
        preferred
    );
    assert_eq!(
        Config::validate_learning_dictionary(&preferred).unwrap(),
        (preferred.clone(), false)
    );
    fs::remove_file(&preferred).unwrap();
    let mut config = dir.all_backends(None);
    assert_eq!(
        config.include_default_learning_dictionary(&dir.0).unwrap(),
        None
    );
    assert_eq!(
        Config::validate_learning_dictionary(&preferred).unwrap(),
        (preferred.clone(), true)
    );
    fs::create_dir(&preferred).unwrap();
    assert!(Config::default_learning_dictionary(&dir.0).is_err());
    assert!(Config::validate_learning_dictionary(&preferred).is_err());
}

#[test]
fn invalid_preferred_is_an_error_even_when_legacy_exists() {
    for legacy in [false, true] {
        let dir = TempDir::new();
        fs::create_dir_all(dir.path(PREFERRED)).unwrap();
        if legacy {
            write_default(&dir, LEGACY);
        }
        assert!(Config::default_learning_dictionary(&dir.0).is_err());
        assert!(dir.path(PREFERRED).is_dir());
        assert_eq!(dir.path(LEGACY).exists(), legacy);
    }
}

#[test]
fn invalid_legacy_is_an_error_only_when_preferred_is_absent() {
    let dir = TempDir::new();
    fs::create_dir_all(dir.path(LEGACY)).unwrap();
    assert!(Config::default_learning_dictionary(&dir.0).is_err());
    let preferred = write_default(&dir, PREFERRED);
    assert_eq!(
        Config::default_learning_dictionary(&dir.0).unwrap(),
        preferred
    );
    assert!(dir.path(LEGACY).is_dir());
}

#[test]
fn malformed_default_parent_is_not_treated_as_absence() {
    let dir = TempDir::new();
    write_default(&dir, LEGACY);
    fs::write(dir.path(".config/spellcheck"), "not a directory").unwrap();
    assert!(Config::default_learning_dictionary(&dir.0).is_err());
    assert_eq!(
        fs::read_to_string(dir.path(".config/spellcheck")).unwrap(),
        "not a directory"
    );
}

#[cfg(unix)]
#[test]
fn default_selection_canonicalizes_file_symlinks_and_rejects_broken_or_directory_links() {
    use std::os::unix::fs::symlink;

    for location in [PREFERRED, LEGACY] {
        for target in ["learning.dic", "missing.dic", "nested"] {
            let dir = TempDir::new();
            let path = dir.path(location);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            symlink(dir.path(target), &path).unwrap();
            if location == PREFERRED {
                write_default(&dir, LEGACY);
            }
            let selected = Config::default_learning_dictionary(&dir.0);
            if target == "learning.dic" {
                assert_eq!(selected.unwrap(), dir.path(target));
            } else {
                assert!(selected.is_err(), "{location}: {target}");
            }
            assert!(fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink());
            assert!(!dir.path("missing.dic").exists());
        }
    }
}

#[cfg(unix)]
#[test]
fn default_selection_propagates_symlink_resolution_errors() {
    let dir = TempDir::new();
    write_default(&dir, LEGACY);
    fs::create_dir_all(dir.path(".config/spellcheck")).unwrap();
    std::os::unix::fs::symlink("learnings.dic", dir.path(PREFERRED)).unwrap();
    assert!(Config::default_learning_dictionary(&dir.0).is_err());
}

#[cfg(unix)]
#[test]
fn default_selection_propagates_read_permission_errors() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new();
    let preferred = write_default(&dir, PREFERRED);
    write_default(&dir, LEGACY);
    fs::set_permissions(&preferred, std::fs::Permissions::from_mode(0o000)).unwrap();
    // Root and processes with DAC overrides can still read mode-000 files.
    if fs::File::open(&preferred).is_err() {
        assert!(Config::default_learning_dictionary(&dir.0).is_err());
    }
    fs::set_permissions(&preferred, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[cfg(unix)]
#[test]
fn readonly_defaults_remain_available_for_read_only_discovery() {
    use std::os::unix::fs::PermissionsExt;

    for location in [PREFERRED, LEGACY] {
        let dir = TempDir::new();
        let path = write_default(&dir, location);
        fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        assert_eq!(Config::default_learning_dictionary(&dir.0).unwrap(), path);
        assert!(Config::validate_learning_dictionary(&path).is_err());
        let mut config = dir.all_backends(None);
        assert_eq!(
            config.include_default_learning_dictionary(&dir.0).unwrap(),
            (!BACKENDS.is_empty()).then(|| path.clone())
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "1\nZzyzxqvocab\n");
        fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

#[test]
fn discovery_loads_only_existing_defaults_into_all_active_backends_and_deduplicates() {
    for location in [PREFERRED, LEGACY] {
        let dir = TempDir::new();
        let path = write_default(&dir, location);
        let mut config = dir.all_backends(None);
        for _ in 0..2 {
            assert_eq!(
                config.include_default_learning_dictionary(&dir.0).unwrap(),
                (!BACKENDS.is_empty()).then(|| path.clone())
            );
        }
        for &(_, backend) in BACKENDS {
            let spelling = backend(&mut config).as_ref().unwrap();
            assert_eq!(
                spelling.extra_dictionaries,
                vec![dir.path("learning.dic"), path.clone()]
            );
            assert!(spelling.learning_dictionary.is_none());
        }
        assert!(!config.to_toml().unwrap().contains("learning_dictionary"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "1\nZzyzxqvocab\n");
        assert_eq!(dir.path(PREFERRED).exists(), location == PREFERRED);
        assert_eq!(dir.path(LEGACY).exists(), location == LEGACY);
    }
}

#[test]
fn missing_default_discovery_does_not_change_config_or_create_directories() {
    let dir = TempDir::new();
    let mut config = dir.all_backends(None);
    let before = config.to_toml().unwrap();
    assert_eq!(
        config.include_default_learning_dictionary(&dir.0).unwrap(),
        None
    );
    assert_eq!(config.to_toml().unwrap(), before);
    assert!(!dir.path(".config").exists());
}

#[test]
fn discovery_propagates_invalid_default_errors_without_changing_config() {
    let dir = TempDir::new();
    write_default(&dir, LEGACY);
    fs::create_dir_all(dir.path(PREFERRED)).unwrap();
    let mut config = dir.all_backends(None);
    let before = config.to_toml().unwrap();
    let result = config.include_default_learning_dictionary(&dir.0);
    if BACKENDS.is_empty() {
        assert_eq!(result.unwrap(), None);
    } else {
        assert!(result.is_err());
    }
    assert_eq!(config.to_toml().unwrap(), before);
    assert!(dir.path(PREFERRED).is_dir());
    assert_eq!(
        fs::read_to_string(dir.path(LEGACY)).unwrap(),
        "1\nZzyzxqvocab\n"
    );
}

#[test]
fn explicit_learning_settings_skip_discovery_and_strict_validation() {
    let dir = TempDir::new();
    fs::create_dir_all(dir.path(PREFERRED)).unwrap();
    for &(_, selecting) in BACKENDS {
        for target in ["missing.dic", "nested", "first.dic"] {
            let mut config = dir.all_backends(Some("learning.dic"));
            selecting(&mut config).as_mut().unwrap().learning_dictionary = Some(dir.path(target));
            let before = config.to_toml().unwrap();
            assert_eq!(
                config.include_default_learning_dictionary(&dir.0).unwrap(),
                None
            );
            assert_eq!(config.to_toml().unwrap(), before);
        }
    }
}

#[test]
fn default_discovery_does_not_validate_mixed_languages() {
    let dir = TempDir::new();
    let path = write_default(&dir, PREFERRED);
    for (index, &(_, backend)) in BACKENDS.iter().enumerate() {
        for &(_, other) in &BACKENDS[index + 1..] {
            let mut config = disabled_config();
            *backend(&mut config) = Some(dir.spelling_config(None, &[]));
            let mut spelling = dir.spelling_config(None, &[]);
            spelling.lang = toml::from_str::<HunspellConfig>("lang = 'de_DE'")
                .unwrap()
                .lang;
            *other(&mut config) = Some(spelling);
            assert!(config.learning_dictionary().is_err());
            assert_eq!(
                config.include_default_learning_dictionary(&dir.0).unwrap(),
                Some(path.clone())
            );
        }
    }
}

#[test]
fn no_active_backend_skips_default_filesystem_validation() {
    let dir = TempDir::new();
    fs::create_dir_all(dir.path(PREFERRED)).unwrap();
    let mut config = disabled_config();
    assert!(!config.has_spelling_backends());
    let before = config.to_toml().unwrap();
    assert_eq!(
        config.include_default_learning_dictionary(&dir.0).unwrap(),
        None
    );
    config.add_learning_dictionary(&dir.path("learning.dic"));
    assert_eq!(config.to_toml().unwrap(), before);
}

#[test]
fn registration_only_updates_active_compiled_backends_without_changing_explicit_targets() {
    let dir = TempDir::new();
    let mut config = dir.all_backends(Some("learning.dic"));
    assert_eq!(config.has_spelling_backends(), !BACKENDS.is_empty());
    let path = dir.path("other.dic");
    config.add_learning_dictionary(&path);
    config.add_learning_dictionary(&path);
    for &(_, backend) in BACKENDS {
        let spelling = backend(&mut config).as_ref().unwrap();
        assert_eq!(spelling.learning_dictionary, Some(dir.path("learning.dic")));
        assert_eq!(
            spelling.extra_dictionaries,
            vec![dir.path("learning.dic"), path.clone()]
        );
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), "0\n");
}

#[cfg(not(all(feature = "hunspell", feature = "zet", feature = "spellbook")))]
#[test]
fn uncompiled_backends_do_not_suppress_discovery_or_receive_registrations() {
    let dir = TempDir::new();
    let path = write_default(&dir, PREFERRED);
    let uncompiled: &[Backend] = &[
        #[cfg(not(feature = "hunspell"))]
        |config| &mut config.hunspell,
        #[cfg(not(feature = "zet"))]
        |config| &mut config.zet,
        #[cfg(not(feature = "spellbook"))]
        |config| &mut config.spellbook,
    ];
    let mut config = dir.all_backends(None);
    for backend in uncompiled {
        *backend(&mut config) = Some(dir.spelling_config(Some("missing.dic"), &[]));
    }
    assert_eq!(config.has_spelling_backends(), !BACKENDS.is_empty());
    assert_eq!(
        config.include_default_learning_dictionary(&dir.0).unwrap(),
        (!BACKENDS.is_empty()).then(|| path.clone())
    );
    config.add_learning_dictionary(&path);
    let missing = dir.path("missing.dic");
    for backend in uncompiled {
        backend(&mut config)
            .as_mut()
            .unwrap()
            .extra_dictionaries
            .push(missing.clone());
    }
    config.omit_missing_learning_dictionary(&missing).unwrap();
    for backend in uncompiled {
        let spelling = backend(&mut config).as_ref().unwrap();
        assert_eq!(spelling.extra_dictionaries, vec![missing.clone()]);
        assert_eq!(spelling.learning_dictionary, Some(dir.path("missing.dic")));
    }
}
