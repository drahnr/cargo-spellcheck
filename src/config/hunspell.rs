//! Hunspell checker configuration.

use super::{Lang5, SearchDirs, WrappedRegex};
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
        self.search_dirs = self
            .search_dirs
            .iter(!self.skip_os_lookups)
            .filter_map(|search_dir| {
                let abspath = if !search_dir.is_absolute() {
                    base.join(search_dir)
                } else {
                    search_dir.to_owned()
                };

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
            if extra_dic.is_absolute() {
                let resolved = extra_dic.canonicalize().wrap_err_with(|| {
                    format!("Could not resolve extra dictionary {}", extra_dic.display())
                })?;
                if !resolved.is_file() {
                    bail!(
                        "Extra dictionary {} must be a regular file",
                        extra_dic.display()
                    );
                }
                *extra_dic = resolved;
                continue;
            }

            let resolved = self
                .search_dirs
                .iter(!self.skip_os_lookups)
                .find_map(|search_dir| {
                    let candidate = search_dir.join(&*extra_dic);
                    match candidate.canonicalize() {
                        Ok(path) => path.is_file().then_some(path),
                        Err(_) => {
                            log::debug!("Failed to canonicalize {}", candidate.display());
                            None
                        }
                    }
                });
            if let Some(resolved) = resolved {
                *extra_dic = resolved;
            } else {
                bail!(
                    "Could not find extra dictionary {} in any of the search paths; expected a regular file",
                    extra_dic.display()
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "cargo-spellcheck-extra-dictionary-paths-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }

        fn config(&self, dictionary: PathBuf, search_dirs: &[&str]) -> HunspellConfig {
            HunspellConfig {
                extra_dictionaries: vec![dictionary],
                search_dirs: search_dirs
                    .iter()
                    .map(PathBuf::from)
                    .collect::<Vec<_>>()
                    .into(),
                skip_os_lookups: true,
                ..Default::default()
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn absolute_dictionary_is_canonicalized_independently_of_search_directories() {
        let fixture = Fixture::new();
        fs::create_dir(fixture.0.join("nested")).unwrap();
        fs::create_dir(fixture.0.join("search")).unwrap();
        fs::write(fixture.0.join("extra.dic"), "1\nAbsoluteword\n").unwrap();
        fs::write(fixture.0.join("search/extra.dic"), "1\nSearchword\n").unwrap();
        let dictionary = fixture.0.join("nested/../extra.dic");
        let expected = fs::canonicalize(&dictionary).unwrap();

        for search_dirs in [&[][..], &["missing"][..], &["search"][..]] {
            let mut config = fixture.config(dictionary.clone(), search_dirs);
            config.sanitize_paths(&fixture.0).unwrap();
            assert_eq!(config.extra_dictionaries, vec![expected.clone()]);
        }
    }

    #[test]
    fn missing_absolute_dictionary_is_rejected_even_with_search_directories() {
        let fixture = Fixture::new();
        fs::create_dir(fixture.0.join("search")).unwrap();
        fs::write(fixture.0.join("search/missing.dic"), "0\n").unwrap();
        let dictionary = fixture.0.join("missing.dic");

        for search_dirs in [&[][..], &["search"][..]] {
            let mut config = fixture.config(dictionary.clone(), search_dirs);
            let error = config.sanitize_paths(&fixture.0).unwrap_err();
            assert!(error
                .to_string()
                .contains("Could not resolve extra dictionary"));
            assert!(error
                .to_string()
                .contains(&dictionary.display().to_string()));
            assert_eq!(
                error.downcast_ref::<std::io::Error>().unwrap().kind(),
                std::io::ErrorKind::NotFound
            );
        }
        assert!(!dictionary.exists());
    }

    #[test]
    fn absolute_directory_is_rejected_even_with_search_directories() {
        let fixture = Fixture::new();
        let dictionary = fixture.0.join("extra.dic");
        fs::create_dir(&dictionary).unwrap();

        for search_dirs in [&[][..], &["."][..]] {
            let mut config = fixture.config(dictionary.clone(), search_dirs);
            let error = config.sanitize_paths(&fixture.0).unwrap_err().to_string();
            assert!(error.contains("must be a regular file"), "{error}");
            assert!(error.contains(&dictionary.display().to_string()), "{error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn absolute_dictionary_symlinks_are_canonicalized_and_dangling_links_are_rejected() {
        let fixture = Fixture::new();
        let dictionary = fixture.0.join("extra.dic");
        let alias = fixture.0.join("alias.dic");
        fs::write(&dictionary, "0\n").unwrap();
        std::os::unix::fs::symlink(&dictionary, &alias).unwrap();
        let mut config = fixture.config(alias.clone(), &[]);
        config.sanitize_paths(&fixture.0).unwrap();
        assert_eq!(config.extra_dictionaries, vec![dictionary.clone()]);

        fs::remove_file(&dictionary).unwrap();
        let mut config = fixture.config(alias.clone(), &["."]);
        let error = config.sanitize_paths(&fixture.0).unwrap_err().to_string();
        assert!(
            error.contains("Could not resolve extra dictionary"),
            "{error}"
        );
        assert!(error.contains(&alias.display().to_string()), "{error}");
    }

    #[test]
    fn relative_dictionary_search_skips_missing_and_non_file_candidates() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.0.join("first/extra.dic")).unwrap();
        fs::create_dir(fixture.0.join("second")).unwrap();
        fs::write(fixture.0.join("second/extra.dic"), "0\n").unwrap();
        let mut config = fixture.config("extra.dic".into(), &["missing", "first", "second"]);
        config.sanitize_paths(&fixture.0).unwrap();
        assert_eq!(
            config.extra_dictionaries,
            vec![fs::canonicalize(fixture.0.join("second/extra.dic")).unwrap()]
        );
        assert!(!fixture.0.join("missing").exists());
    }

    #[test]
    fn relative_missing_and_non_file_dictionaries_are_rejected() {
        let fixture = Fixture::new();
        fs::create_dir(fixture.0.join("directory.dic")).unwrap();
        for dictionary in ["missing.dic", "directory.dic"] {
            let mut config = fixture.config(dictionary.into(), &["."]);
            let error = config.sanitize_paths(&fixture.0).unwrap_err().to_string();
            assert!(error.contains(dictionary), "{error}");
            assert!(
                error.contains("search paths; expected a regular file"),
                "{error}"
            );
        }
        assert!(!fixture.0.join("missing.dic").exists());
    }
}
