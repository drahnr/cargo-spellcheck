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
    /// the word.
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
                    base.join(&search_dir)
                } else {
                    search_dir.to_owned()
                };

                abspath.canonicalize().ok().map(|abspath| {
                    log::trace!(
                        "Sanitized ({} + {}) -> {}",
                        base.display(),
                        search_dir.display(),
                        abspath.display()
                    );
                    abspath
                })
            })
            .collect::<Vec<PathBuf>>()
            .into();

        // convert all extra dictionaries to absolute paths

        'o: for extra_dic in self.extra_dictionaries.iter_mut() {
            for search_dir in
                self.search_dirs
                    .iter(!self.skip_os_lookups)
                    .filter_map(|search_dir| {
                        if !extra_dic.is_absolute() {
                            base.join(&search_dir).canonicalize().ok()
                        } else {
                            Some(search_dir.to_owned())
                        }
                    })
            {
                let abspath = if !extra_dic.is_absolute() {
                    search_dir.join(&extra_dic)
                } else {
                    continue 'o;
                };
                if let Ok(abspath) = abspath.canonicalize() {
                    if abspath.is_file() {
                        *extra_dic = abspath;
                        continue 'o;
                    }
                } else {
                    log::debug!("Failed to canonicalize {}", abspath.display());
                }
            }
            bail!(
                "Could not find extra dictionary {} in any of the search paths",
                extra_dic.display()
            );
        }

        Ok(())
    }
}
