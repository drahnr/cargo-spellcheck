//! Hunspell checker configuration.

use super::{Config, Lang5, SearchDirs, WrappedRegex};
use std::path::{Path, PathBuf};

use crate::errors::*;

use serde::{Deserialize, Serialize};

const fn yes() -> bool {
    true
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Quirks {
    /// A regular expression, whose capture groups will be checked, instead of
    /// the initial token. Only the first one that matches will be used to split
    /// the word. Expressions without capture groups allow-list matching tokens.
    /// All expressions can also match across two or more tokenizer tokens within
    /// one non-whitespace sequence, without excluding neighbors outside the match.
    /// Without capture groups, the eligible matched portion is excluded. With
    /// capture groups (including named groups), each nonempty participating group
    /// is a separate check candidate, even if it spans tokens; uncaptured portions
    /// of the full match (group 0) are skipped. Compound captures are clipped to
    /// group 0; existing per-token lookaround behavior is unchanged. Empty and
    /// unmatched groups are ignored; if none remain in a compound match, its eligible
    /// portion is skipped. Existing per-token empty-capture behavior is unchanged.
    /// Overlapping or nested captures are independent;
    /// identical capture ranges are deduplicated before recursive transformation.
    /// Rules are ordered: an earlier expression matching an original token protects
    /// that entire token from later cross-token matches. Unprotected tokens can
    /// still be transformed by the same cross-token match. Ranges selected by an
    /// earlier cross-token capture are protected from later cross-token exclusions.
    /// An earlier exclusion is never restored by a later capture rule. Captured
    /// fragments are recursively transformed using the same ordered rules; identity
    /// captures advance to later rules to ensure termination. Captures and remaining
    /// token fragments are transformed as standalone strings, so anchors and
    /// lookarounds see fragment boundaries; recursive rules can still allow-list them.
    #[serde(default)]
    pub transform_regex: Vec<WrappedRegex>,
    /// Allow concatenated words instead of dashed connection. Note that this
    /// only applies, if one of the suggested replacements has an item that is
    /// equivalent except for addition dashes (`-`).
    #[serde(default)]
    pub allow_concatenation: bool,
    /// The counterpart of `allow_concatenation`. Accepts words which have
    /// replacement suggestions that contain additional dashes.
    #[serde(default)]
    pub allow_dashes: bool,
    /// Treats sequences of emojis as OK.
    #[serde(default = "yes")]
    pub allow_emojis: bool,
    /// Check the expressions in the footnote references. By default this is
    /// turned on to remain backwards compatible but disabling it could be
    /// particularly useful when one uses abbreviations instead of numbers as
    /// footnote references.  For instance by default the fragment `hello[^xyz]`
    /// would be spellchecked as `helloxyz` which is obviously a misspelled
    /// word, but by turning this check off, it will skip validating the
    /// reference altogether and will only check the word `hello`.
    #[serde(default = "yes")]
    pub check_footnote_references: bool,
}

impl Default for Quirks {
    fn default() -> Self {
        Self {
            transform_regex: Vec::new(),
            allow_concatenation: false,
            allow_dashes: false,
            allow_emojis: true,
            check_footnote_references: true,
        }
    }
}

#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
impl Quirks {
    pub(crate) const fn allow_concatenated(&self) -> bool {
        self.allow_concatenation
    }

    pub(crate) const fn allow_dashed(&self) -> bool {
        self.allow_dashes
    }

    pub(crate) const fn allow_emojis(&self) -> bool {
        self.allow_emojis
    }

    pub(crate) fn transform_regex(&self) -> &[WrappedRegex] {
        &self.transform_regex
    }

    pub(crate) fn check_footnote_references(&self) -> bool {
        self.check_footnote_references
    }
}

fn default_tokenization_splitchars() -> String {
    "\",;:.!?#(){}[]|/_-‒'`&@§¶…".to_owned()
}

pub type ZetConfig = HunspellConfig;
pub type SpellbookConfig = HunspellConfig;

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct HunspellConfig {
    /// The language we want to check against, used as the dictionary and
    /// affixes file name.
    #[serde(default)]
    pub lang: Lang5,
    /// Additional search directories for `.dic` and `.aff` files.
    // must be option so it can be omitted in the config
    #[serde(default)]
    pub search_dirs: SearchDirs,

    /// Avoid the OS provided dictionaries and only use the builtin ones,
    /// besides those defined in `extra_dictionaries`.
    #[serde(default)]
    pub skip_os_lookups: bool,

    /// Use the builtin dictionaries as last resort. Usually combined with
    /// `skip_os_lookups=true` to enforce the `builtin` usage. Does not prevent
    /// the usage of `extra_dictionaries`.
    #[serde(default)]
    pub use_builtin: bool,

    #[serde(default = "default_tokenization_splitchars")]
    pub tokenization_splitchars: String,

    /// Additional dictionaries for topic specific lingo.
    #[serde(default)]
    pub extra_dictionaries: Vec<PathBuf>,
    /// Extra dictionary to receive words accepted during an interactive run, created if absent.
    /// Relative paths are resolved against the configuration file's directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learning_dictionary: Option<PathBuf>,
    /// Additional quirks besides dictionary lookups.
    #[serde(default)]
    pub quirks: Quirks,
}

impl Default for HunspellConfig {
    fn default() -> Self {
        Self {
            lang: Lang5::en_US,
            search_dirs: SearchDirs::default(),
            extra_dictionaries: Vec::default(),
            learning_dictionary: None,
            quirks: Quirks::default(),
            tokenization_splitchars: default_tokenization_splitchars(),
            skip_os_lookups: false,
            use_builtin: true,
        }
    }
}

impl HunspellConfig {
    pub fn lang(&self) -> Lang5 {
        self.lang
    }

    pub fn search_dirs(&self) -> impl Iterator<Item = &PathBuf> {
        self.search_dirs.iter(!self.skip_os_lookups)
    }

    pub fn extra_dictionaries(&self) -> impl Iterator<Item = &PathBuf> {
        self.extra_dictionaries.iter()
    }

    pub fn sanitize_paths(&mut self, base: &Path) -> Result<()> {
        let targets = self
            .learning_dictionary
            .iter()
            .map(|path| base.join(path))
            .collect::<Vec<_>>();
        self.sanitize_paths_with_learning_targets(base, &targets)
    }

    pub(super) fn sanitize_paths_with_learning_targets(
        &mut self,
        base: &Path,
        learning_targets: &[PathBuf],
    ) -> Result<()> {
        // Resolution errors for declared targets are intentionally deferred until CLI filtering.
        // Keep both spellings so invalid declarations and equivalent missing paths remain registered.
        let learning_targets = learning_targets
            .iter()
            .map(|path| (path, Config::resolve_dictionary_path(path)))
            .collect::<Vec<_>>();
        self.search_dirs = self
            .search_dirs
            .iter(!self.skip_os_lookups)
            .filter_map(|search_dir| {
                let abspath = base.join(search_dir);

                abspath.canonicalize().ok().inspect(|abspath| {
                    log::trace!(
                        "Sanitized ({} + {}) -> {}",
                        base.display(),
                        search_dir.display(),
                        abspath.display()
                    );
                })
            })
            .collect::<Vec<PathBuf>>()
            .into();

        // convert all extra dictionaries to absolute paths

        for extra_dic in &mut self.extra_dictionaries {
            // Search failures are intentional: a later search directory may contain the file.
            let resolved = if extra_dic.is_absolute() {
                extra_dic.canonicalize().ok().filter(|path| path.is_file())
            } else {
                self.search_dirs
                    .iter(!self.skip_os_lookups)
                    .find_map(|search_dir| {
                        let abspath = search_dir.join(&*extra_dic);
                        match abspath.canonicalize() {
                            Ok(path) => path.is_file().then_some(path),
                            Err(_) => {
                                log::debug!("Failed to canonicalize {}", abspath.display());
                                None
                            }
                        }
                    })
            };
            if let Some(resolved) = resolved {
                *extra_dic = resolved;
                continue;
            }
            let candidate = base.join(&*extra_dic);
            if let Some((_, resolved)) = learning_targets
                .iter()
                .find(|(path, _)| **path == candidate)
            {
                *extra_dic = resolved.as_ref().unwrap_or(&candidate).clone();
                continue;
            }
            let resolved = Config::resolve_dictionary_path(&candidate).wrap_err_with(|| {
                format!("Could not find extra dictionary {}", candidate.display())
            })?;
            if learning_targets
                .iter()
                .any(|(_, target)| matches!(target, Ok(target) if target == &resolved))
            {
                *extra_dic = resolved;
                continue;
            }
            bail!(
                "Could not find extra dictionary {} in any of the search paths; expected a regular file",
                candidate.display()
            );
        }

        // Validate learning targets only after CLI filtering has selected the active backends.
        if let Some(configured) = self.learning_dictionary.as_mut() {
            if !configured.is_absolute() {
                *configured = base.join(&*configured);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs_err as fs;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "cargo-spellcheck-learning-config-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn config(&self) -> HunspellConfig {
            HunspellConfig {
                search_dirs: vec![PathBuf::from(".")].into(),
                skip_os_lookups: true,
                extra_dictionaries: vec![PathBuf::from("lingo.dic")],
                learning_dictionary: Some(PathBuf::from("lingo.dic")),
                ..HunspellConfig::default()
            }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn learning_is_opt_in_and_omitted_when_unconfigured() {
        assert!(HunspellConfig::default().learning_dictionary.is_none());
        let config: HunspellConfig = toml::from_str("").unwrap();
        assert!(config.learning_dictionary.is_none());
        assert!(!toml::to_string(&config)
            .unwrap()
            .contains("learning_dictionary"));
        let config: HunspellConfig = toml::from_str("learning_dictionary = 'lingo.dic'").unwrap();
        assert_eq!(config.learning_dictionary, Some(PathBuf::from("lingo.dic")));
    }

    #[test]
    fn resolves_learning_dictionary_relative_to_configuration() {
        let dir = TempDir::new();
        fs::write(dir.0.join("lingo.dic"), "0\n").unwrap();
        let mut config = dir.config();
        config.sanitize_paths(&dir.0).unwrap();
        let expected = fs::canonicalize(dir.0.join("lingo.dic")).unwrap();
        assert_eq!(config.learning_dictionary.as_ref(), Some(&expected));
        assert_eq!(config.extra_dictionaries, vec![expected]);
    }

    #[test]
    fn preserves_absolute_learning_dictionary_until_validation() {
        let dir = TempDir::new();
        fs::write(dir.0.join("lingo.dic"), "0\n").unwrap();
        let mut config = dir.config();
        config.learning_dictionary = Some(dir.0.join(".").join("lingo.dic"));
        config.sanitize_paths(&dir.0).unwrap();
        assert_eq!(
            config.learning_dictionary,
            Some(dir.0.join(".").join("lingo.dic"))
        );
    }

    #[test]
    fn does_not_guess_the_first_extra_dictionary() {
        let dir = TempDir::new();
        fs::write(dir.0.join("lingo.dic"), "0\n").unwrap();
        let mut config = dir.config();
        config.learning_dictionary = None;
        config.sanitize_paths(&dir.0).unwrap();
        assert!(config.learning_dictionary.is_none());
    }

    #[test]
    fn defers_learning_dictionary_membership_validation() {
        let dir = TempDir::new();
        fs::write(dir.0.join("lingo.dic"), "0\n").unwrap();
        fs::write(dir.0.join("other.dic"), "0\n").unwrap();
        let mut config = dir.config();
        config.learning_dictionary = Some(PathBuf::from("other.dic"));
        config.sanitize_paths(&dir.0).unwrap();
        assert_eq!(config.learning_dictionary, Some(dir.0.join("other.dic")));
    }

    #[test]
    fn missing_learning_target_is_not_searched_for_or_created() {
        let dir = TempDir::new();
        fs::create_dir(dir.0.join("search")).unwrap();
        fs::write(dir.0.join("search/lingo.dic"), "0\n").unwrap();
        let mut config = dir.config();
        config.search_dirs = vec![PathBuf::from("search")].into();
        config.sanitize_paths(&dir.0).unwrap();
        assert_eq!(config.learning_dictionary, Some(dir.0.join("lingo.dic")));
        assert_eq!(
            config.extra_dictionaries,
            vec![fs::canonicalize(dir.0.join("search/lingo.dic")).unwrap()]
        );
        assert!(!dir.0.join("lingo.dic").exists());
    }

    #[test]
    fn searches_past_missing_directories_and_non_file_candidates() {
        let dir = TempDir::new();
        fs::create_dir_all(dir.0.join("first/lingo.dic")).unwrap();
        fs::create_dir(dir.0.join("second")).unwrap();
        fs::write(dir.0.join("second/lingo.dic"), "0\n").unwrap();
        let mut config = dir.config();
        config.learning_dictionary = None;
        config.search_dirs = vec!["missing".into(), "first".into(), "second".into()].into();
        config.sanitize_paths(&dir.0).unwrap();
        assert_eq!(
            config.extra_dictionaries,
            vec![dir.0.join("second/lingo.dic").canonicalize().unwrap()]
        );
        assert!(!dir.0.join("missing").exists());
    }

    #[test]
    fn deferred_target_errors_do_not_hide_invalid_ordinary_registrations() {
        let dir = TempDir::new();
        fs::write(dir.0.join("lingo.dic"), "0\n").unwrap();
        let mut config = dir.config();
        config.learning_dictionary = Some("lingo.dic/child.dic".into());
        config.extra_dictionaries =
            vec!["lingo.dic/child.dic".into(), "ordinary-missing.dic".into()];
        let error = config.sanitize_paths(&dir.0).unwrap_err().to_string();
        assert!(error.contains("ordinary-missing.dic"), "{error}");
        assert!(!dir.0.join("ordinary-missing.dic").exists());
    }

    #[test]
    fn defers_learning_dictionary_file_type_validation() {
        let dir = TempDir::new();
        fs::write(dir.0.join("lingo.dic"), "0\n").unwrap();
        fs::create_dir(dir.0.join("not-a-dictionary")).unwrap();
        let mut config = dir.config();
        config.learning_dictionary = Some(PathBuf::from("not-a-dictionary"));
        config.sanitize_paths(&dir.0).unwrap();
        assert_eq!(
            config.learning_dictionary,
            Some(dir.0.join("not-a-dictionary"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn defers_learning_dictionary_symlink_resolution() {
        let dir = TempDir::new();
        fs::write(dir.0.join("lingo.dic"), "0\n").unwrap();
        std::os::unix::fs::symlink(dir.0.join("lingo.dic"), dir.0.join("alias.dic")).unwrap();
        let mut config = dir.config();
        config.learning_dictionary = Some(PathBuf::from("alias.dic"));
        config.sanitize_paths(&dir.0).unwrap();
        assert_eq!(config.learning_dictionary, Some(dir.0.join("alias.dic")));
    }
}
