//! Configure cargo-spellcheck
//!
//! Supports `Hunspell` and `LanguageTool` scopes.
//!
//! A default configuration will be generated in the default
//! location by default. Default. Default default default.

// TODO pendeng refactor, avoid spending time on documenting the status quo.
#![allow(missing_docs)]

mod search_dirs;
pub use search_dirs::*;

use crate::suggestion::Detector;
use crate::reflow::ReflowConfig;
use crate::suggestion::Detector;
use anyhow::{anyhow, bail, Error, Result};
use fancy_regex::Regex;
use log::trace;

use serde::{Deserialize, Serialize};
use std::convert::AsRef;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(alias = "Hunspell")]
    pub hunspell: Option<HunspellConfig>,
    #[serde(alias = "LanguageTool")]
    #[serde(alias = "languageTool")]
    #[serde(alias = "Languagetool")]
    pub languagetool: Option<LanguageToolConfig>,
    reflow: Option<ReflowConfig>,
}

#[derive(Debug)]
pub struct WrappedRegex(pub Regex);

impl Clone for WrappedRegex {
    fn clone(&self) -> Self {
        // TODO inefficient.. but right now this should almost never happen
        // TODO implement a lazy static `Arc<Mutex<HashMap<&'static str,Regex>>`
        Self(Regex::new(self.as_str()).unwrap())
    }
}

impl std::ops::Deref for WrappedRegex {
    type Target = Regex;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::convert::AsRef<Regex> for WrappedRegex {
    fn as_ref(&self) -> &Regex {
        &self.0
    }
}

impl Serialize for WrappedRegex {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WrappedRegex {
    fn deserialize<D>(deserializer: D) -> Result<WrappedRegex, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        deserializer
            .deserialize_any(RegexVisitor)
            .map(WrappedRegex::from)
    }
}

impl Into<Regex> for WrappedRegex {
    fn into(self) -> Regex {
        self.0
    }
}

impl From<Regex> for WrappedRegex {
    fn from(other: Regex) -> WrappedRegex {
        WrappedRegex(other)
    }
}

struct RegexVisitor;

impl<'de> serde::de::Visitor<'de> for RegexVisitor {
    type Value = Regex;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("String with valid regex expression")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        let re = Regex::new(value).map_err(E::custom)?;
        Ok(re)
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_str::<E>(value.as_str())
    }
}

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
    /// The counterpart of `allow_concatenation`. Accepts words which have repalcement suggestions
    /// that contain additional dashes.
    #[serde(default)]
    pub allow_dashes: bool,
}

impl Default for Quirks {
    fn default() -> Self {
        // use some for default, so for generating the default config has the default values
        // but the options are necessary to allow omitting them in the config file
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

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct HunspellConfig {
    /// The language we want to check against, used as the dictionary and affixes file name.
    // TODO impl a custom xx_YY code deserializer based on iso crates
    pub lang: Option<String>,
    /// Additional search dirs for `.dic` and `.aff` files.
    // must be option so it can be omitted in the config
    #[serde(default)]
    pub search_dirs: SearchDirs,
    /// Additional dictionaries for topic specific lingo.
    #[serde(default)]
    pub extra_dictionaries: Vec<PathBuf>,
    /// Additional quirks besides dictionary lookups.
    #[serde(default)]
    pub quirks: Quirks,
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
                    trace!(
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

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct LanguageToolConfig {
    pub url: url::Url,
}

impl LanguageToolConfig {
    pub fn url(&self) -> &url::Url {
        &self.url
    }
}

impl Config {
    const QUALIFIER: &'static str = "io";
    const ORGANIZATION: &'static str = "spearow";
    const APPLICATION: &'static str = "cargo_spellcheck";

    /// Sanitize all relative paths to absolute paths
    /// in relation to `base`.
    fn sanitize_paths(&mut self, base: &Path) -> Result<()> {
        if let Some(ref mut hunspell) = self.hunspell {
            hunspell.sanitize_paths(base)?;
        }
        Ok(())
    }

    pub fn parse<S: AsRef<str>>(s: S) -> Result<Self> {
        Ok(toml::from_str(s.as_ref())?)
    }

    pub fn load_from<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut file = File::open(path.as_ref().to_str().unwrap())
            .map_err(|e| anyhow!("Failed to open file {}", path.as_ref().display()).context(e))?;
        let mut contents = String::with_capacity(1024);
        file.read_to_string(&mut contents).map_err(|e| {
            anyhow!("Failed to read from file {}", path.as_ref().display()).context(e)
        })?;
        Self::parse(&contents)
            .map_err(|e| {
                e.context(anyhow::anyhow!(
                    "Syntax of a given config file({}) is broken",
                    path.as_ref().display()
                ))
            })
            .and_then(|mut cfg| {
                if let Some(base) = path.as_ref().parent() {
                    cfg.sanitize_paths(base)?;
                }
                Ok(cfg)
            })
    }

    pub fn load() -> Result<Self> {
        if let Some(base) = directories::BaseDirs::new() {
            Self::load_from(
                base.config_dir()
                    .join("cargo_spellcheck")
                    .join("config.toml"),
            )
        } else {
            bail!("No idea where your config directory is located. XDG compliance would be nice.")
        }
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string(self).map_err(|e| anyhow!("Failed to convert to toml").context(e))
    }

    pub fn write_values_to_path<P: AsRef<Path>>(&self, path: P) -> Result<Self> {
        let s = self.to_toml()?;
        let path = path.as_ref();

        if let Some(path) = path.parent() {
            std::fs::create_dir_all(path).map_err(|e| {
                anyhow!("Failed to create directories {}", path.display()).context(e)
            })?;
        }

        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .map_err(|e| {
                anyhow!("Failed to write default values to {}", path.display()).context(e)
            })?;
        let mut writer = std::io::BufWriter::new(file);

        writer.write_all(s.as_bytes()).map_err(|e| {
            anyhow!("Failed to write default config to {}", path.display()).context(e)
        })?;

        Ok(self.clone())
    }

    pub fn write_values_to_default_path(&self) -> Result<Self> {
        let path = Self::default_path()?;
        self.write_values_to_path(path)
    }

    pub fn write_default_values_to<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::default().write_values_to_path(path)
    }

    pub fn default_path() -> Result<PathBuf> {
        if let Some(base) =
            directories::ProjectDirs::from(Self::QUALIFIER, Self::ORGANIZATION, Self::APPLICATION)
        {
            Ok(base.config_dir().join("config.toml"))
        } else {
            bail!("No idea where your config directory is located. `$HOME` must be set.")
        }
    }

    /// Obtain a project specific config file.
    pub fn project_config(manifest_dir: impl AsRef<Path>) -> Result<PathBuf> {
        let path = manifest_dir
            .as_ref()
            .to_owned()
            .join(".config")
            .join("spellcheck.toml");

        let path = path.canonicalize()?;

        if path.is_file() {
            Ok(path)
        } else {
            bail!(
                "Local project dir config {} does not exist or is not a file.",
                path.display()
            )
        }
    }

    pub fn write_default_values() -> Result<Self> {
        let d = Self::default_path()?;
        Self::write_default_values_to(d.join("config.toml"))
    }

    pub fn is_enabled(&self, detector: Detector) -> bool {
        match detector {
            Detector::Hunspell => self.hunspell.is_some(),
            Detector::LanguageTool => self.languagetool.is_some(),
            Detector::Reflow => self.reflow.is_some(),
            #[cfg(test)]
            Detector::Dummy => true,
        }
    }

    pub fn full() -> Self {
        let languagetool = LanguageToolConfig {
            url: url::Url::parse("http://127.0.0.1:8010").expect("Default ip must be ok"),
        };
        Self {
            languagetool: Some(languagetool),
            ..Default::default()
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hunspell: Some(HunspellConfig {
                lang: Some("en_US".to_owned()),
                search_dirs: SearchDirs::default(),
                extra_dictionaries: Vec::new(),
                quirks: Quirks::default(),
            }),
            languagetool: None,
            reflow: None,
        }
    }
}

// TODO figure out which ISO spec this actually is
pub struct CommonLang(String);

impl std::str::FromStr for CommonLang {
    type Err = Error;
    fn from_str(_s: &str) -> std::result::Result<Self, Self::Err> {
        //
        unimplemented!("Common Lang needs a ref spec")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all() {
        let _ = Config::parse(
            r#"
[LanguageTool]
url = "http://127.0.0.1:8010/"

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
lang = "en_US"
search_dirs = ["/usr/lib64/hunspell"]
extra_dictionaries = ["/home/bernhard/test.dic"]
			"#,
        )
        .unwrap();
    }

    #[test]
    fn partial_2() {
        assert!(Config::parse(
            r#"
[languageTool]


[Hunspell]
lang = "en_US"
search_dirs = ["/usr/lib64/hunspell"]
extra_dictionaries = ["/home/bernhard/test.dic"]
			"#,
        )
        .is_err());
    }

    #[test]
    fn partial_3() {
        let cfg = Config::parse(
            r#"
[Hunspell]
lang = "en_US"
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
[LanguageTool]
url = "http://127.0.0.1:8010/"

[Hunspell]
lang = "en_US"
			"#,
        )
        .unwrap();
        let _hunspell = cfg.hunspell.expect("Must contain hunspell cfg");
        let _langtool = cfg.languagetool.expect("Must contain language tool cfg");
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
			"#,
        )
        .unwrap();

        let hunspell: HunspellConfig = cfg.hunspell.expect("Must contain hunspell cfg");
        let search_dirs = hunspell.search_dirs;
        let search_dirs: Vec<_> = search_dirs.as_ref().clone();
        assert!(!search_dirs.is_empty());

        #[cfg(target_os = "linux")]
        assert_eq!(search_dirs.len(), 5);

        #[cfg(target_os = "windows")]
        assert_eq!(search_dirs.len(), 2);

        #[cfg(target_os = "macos")]
        assert!(search_dirs.len() >= 3);
    }
}
