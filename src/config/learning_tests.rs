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
fn active_learning_targets_must_be_existing_files() {
    let dir = TempDir::new();
    for &(name, backend) in BACKENDS {
        for target in ["missing.dic", "nested"] {
            let mut config = disabled_config();
            *backend(&mut config) = Some(dir.spelling_config(Some(target), &["learning.dic"]));
            let error = config.learning_dictionary().unwrap_err().to_string();
            assert!(error.contains("learning_dictionary"), "{error}");
            assert!(error.contains("existing dictionary file"), "{error}");
            assert!(error.contains(name), "{error}");
        }
    }
    assert!(!dir.path("missing.dic").exists());
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
