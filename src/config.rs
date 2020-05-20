//! Configure cargo-spellcheck
//!
//! Supporst `Hunspell` and `LanguageTool` scopes.
//!
//! A default configuration will be generated in the default
//! location by default. Default. Default default default.

use crate::suggestion::Detector;
use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    pub hunspell: Option<HunspellConfig>,
    pub languagetool: Option<LanguageToolConfig>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct HunspellConfig {
    pub lang: Option<String>, // TODO impl a custom xx_YY code deserializer based on iso crates
    // must be option so it can be omiited
    pub search_dirs: Option<Vec<PathBuf>>,
    pub extra_affixes: Option<Vec<PathBuf>>,
    pub extra_dictonaries: Option<Vec<PathBuf>>,
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
        if let Some(ref search_dirs) = self.search_dirs {
            search_dirs.as_slice()
        } else {
            lazy_static::lazy_static! {
                static ref LOOKUP_DIRS: Vec<PathBuf> = vec![PathBuf::from("/usr/share/myspell")];
            };

            LOOKUP_DIRS.as_slice()
        }
    }

    pub fn extra_affixes(&self) -> &[PathBuf] {
        if let Some(ref extra_affixes) = self.extra_affixes {
            extra_affixes.as_slice()
        } else {
            &[]
        }
    }

    pub fn extra_dictonaries(&self) -> &[PathBuf] {
        if let Some(ref extra_dictonaries) = self.extra_dictonaries {
            extra_dictonaries.as_slice()
        } else {
            &[]
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct LanguageToolConfig {
    pub url: url::Url,
}

impl LanguageToolConfig {
    pub fn url(&self) -> &url::Url {
        &self.url
    }
}

impl Config {
    pub fn parse<S: AsRef<str>>(s: S) -> Result<Self> {
        let cfg = toml::from_str(s.as_ref())?;
        Ok(cfg)
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut file = File::open(path.as_ref().to_str().unwrap())?;
        let mut contents = String::with_capacity(1024);
        file.read_to_string(&mut contents)?;
        Self::parse(&contents)
    }

    pub fn load_from_default() -> Result<Self> {
        if let Some(base) = directories::BaseDirs::new() {
            Self::load(base.config_dir())
        } else {
            Err(anyhow!(
                "No idea where your config directory is located. XDG compliance would be nice."
            ))
        }
    }

    pub fn is_enabled(&self, detector: Detector) -> bool {
        match detector {
            Detector::Hunspell => self.hunspell.is_some(),
            Detector::LanguageTool => self.languagetool.is_some(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hunspell: Some(HunspellConfig {
                lang: Some("en_US".to_owned()),
                search_dirs: Some(Vec::new()),
                extra_affixes: Some(Vec::new()),
                extra_dictonaries: Some(Vec::new()),
            }),
            languagetool: None,
        }
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
url = "192.1.1.11:1337/"

[Hunspell]
lang = "en_US"
search_dirs = ["/usr/lib64/hunspell"]
extra_dictonaries = ["/home/bernhard/test.dic"]
extra_affixes = ["/home/bernhard/test/f.aff"]
			"#,
        )
        .unwrap();
    }

    #[test]
    fn empty() {
        let _ = Config::parse(
            r#"
			"#,
        )
        .unwrap();
    }
    #[test]
    fn partial_1() {
        let _ = Config::parse(
            r#"
[hunspell]
lang = "en_US"
search_dirs = ["/usr/lib64/hunspell"]
extra_dictonaries = ["/home/bernhard/test.dic"]
extra_affixes = ["/home/bernhard/test/f.aff"]
			"#,
        )
        .unwrap();
    }

    #[test]
    fn partial_2() {
        let _ = Config::parse(
            r#"
[languageTool]

[Hunspell]
lang = "en_US"
search_dirs = ["/usr/lib64/hunspell"]
extra_dictonaries = ["/home/bernhard/test.dic"]
extra_affixes = ["/home/bernhard/test/f.aff"]
			"#,
        )
        .unwrap();
    }

    #[test]
    fn partial_3() {
        let _ = Config::parse(
            r#"
[Hunspell]
lang = "en_US"
search_dirs = ["/usr/lib64/hunspell"]
extra_dictonaries = ["/home/bernhard/test.dic"]
extra_affixes = ["/home/bernhard/test/f.aff"]
			"#,
        )
        .unwrap();
    }

    #[test]
    fn partial_4() {
        let _ = Config::parse(
            r#"
[LanguageTool]
url = "192.1.1.11:1337/"

[Hunspell]
lang = "en_US"
			"#,
        )
        .unwrap();
    }

    #[test]
    fn partial_5() {
        let _ = Config::parse(
            r#"
[hUNspell]
lang = "en_US"
			"#,
        )
        .unwrap();
    }

    #[test]
    fn partial_6() {
        let _ = Config::parse(
            r#"
[hunspell]
			"#,
        )
        .unwrap();
    }
}
