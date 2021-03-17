//! Hunspell checker configuration.

use super::{SearchDirs, WrappedRegex};
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Quirks {
    /// A regular expression, whose capture groups will be checked, instead of the initial token.
    /// Only the first one that matches will be used to split the word.
    #[serde(default)]
    pub transform_regex: Vec<WrappedRegex>,
    /// Allow concatenated words instead of dashed connection.
    /// Note that this only applies, if one of the suggested replacements has an item that is
    /// equivalent except for addition dashes (`-`).
    #[serde(default)]
    pub allow_concatenation: bool,
    /// The counterpart of `allow_concatenation`. Accepts words which have replacement suggestions
    /// that contain additional dashes.
    #[serde(default)]
    pub allow_dashes: bool,
}

impl Default for Quirks {
    fn default() -> Self {
        Self {
            transform_regex: vec![],
            allow_concatenation: false,
            allow_dashes: false,
        }
    }
}

impl Quirks {
    pub(crate) fn allow_concatenated(&self) -> bool {
        self.allow_concatenation
    }

    pub(crate) fn allow_dashed(&self) -> bool {
        self.allow_dashes
    }

    pub(crate) fn transform_regex(&self) -> &[WrappedRegex] {
        &self.transform_regex
    }
}

fn default_tokenization_splitchars() -> String {
    "\";:,?!#(){}[]\n\r/`".to_owned()
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct HunspellConfig {
    /// The language we want to check against, used as the dictionary and affixes file name.
    // TODO impl a custom xx_YY code deserializer based on iso crates
    pub lang: Option<String>,
    /// Additional search directories for `.dic` and `.aff` files.
    // must be option so it can be omitted in the config
    #[serde(default)]
    pub search_dirs: SearchDirs,
    /// Additional dictionaries for topic specific lingo.
    #[serde(default)]
    pub extra_dictionaries: Vec<PathBuf>,
    /// Additional quirks besides dictionary lookups.
    #[serde(default)]
    pub quirks: Quirks,


    #[serde(default = "default_tokenization_splitchars")]
    pub tokenization_splitchars: String,
}

impl Default for HunspellConfig {
    fn default() -> Self {
        Self {
            lang: Some("en".to_owned()),
            search_dirs: SearchDirs::default(),
            extra_dictionaries: Vec::default(),
            quirks: Quirks::default(),
            tokenization_splitchars: default_tokenization_splitchars(),
        }
    }
}

impl HunspellConfig {
    pub fn lang(&self) -> &str {
        if let Some(ref lang) = self.lang {
            lang.as_str()
        } else {
            "en_US"
        }
    }

    pub fn search_dirs(&self) -> &[PathBuf] {
        &self.search_dirs
    }

    pub fn extra_dictionaries(&self) -> &[PathBuf] {
        &self.extra_dictionaries
    }

    pub fn sanitize_paths(&mut self, base: &Path) -> Result<()> {
        self.search_dirs = self
            .search_dirs
            .iter()
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
            for search_dir in self.search_dirs.iter().filter_map(|search_dir| {
                if !extra_dic.is_absolute() {
                    base.join(&search_dir).canonicalize().ok()
                } else {
                    Some(search_dir.to_owned())
                }
            }) {
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
