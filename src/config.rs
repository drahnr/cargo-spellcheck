//! Configure cargo-spellcheck
//!
//! Supporst `Hunspell` and `LanguageTool` scopes.
//!
//! A default configuration will be generated in the default
//! location by default. Default. Default default default.

use crate::suggestion::Detector;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Config {
    pub hunspell: Option<HunspellConfig>,
    pub languagetool: Option<LanguageToolConfig>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
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

#[derive(Deserialize, Serialize, Debug, Clone)]
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

    pub fn load_from<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut file = File::open(path.as_ref().to_str().unwrap())?;
        let mut contents = String::with_capacity(1024);
        file.read_to_string(&mut contents)?;
        Self::parse(&contents)
    }

    pub fn load() -> Result<Self> {
        if let Some(base) = directories::BaseDirs::new() {
            Self::load_from(
                base.config_dir()
                    .join("cargo_spellcheck")
                    .with_file_name("config.toml"),
            )
        } else {
            Err(anyhow!(
                "No idea where your config directory is located. XDG compliance would be nice."
            ))
        }
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string(self).map_err(|_e| anyhow::anyhow!("Failed to convert to toml"))
    }

    pub fn write_default_values_to<P: AsRef<Path>>(path: P) -> Result<Self> {
        let values = Self::default();

        let s = values.to_toml()?;

        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path.as_ref())?;
        let mut writer = std::io::BufWriter::new(file);

        writer
            .write_all(s.as_bytes())
            .map_err(|_e| anyhow::anyhow!("Failed to write all to {}", path.as_ref().display()))?;

        Ok(values)
    }

    pub fn write_default_values() -> Result<Self> {
        if let Some(base) = directories::BaseDirs::new() {
            let d = base.config_dir().join("cargo_spellcheck");
            std::fs::create_dir_all(d.as_path())?;
            Self::write_default_values_to(d.join("config.toml"))
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
        let mut search_dirs = if cfg!(target_os = "macos") {
            directories::BaseDirs::new()
                .map(|base| vec![base.home_dir().to_owned().join("/Library/Spelling/")])
                .unwrap_or_else(|| Vec::with_capacity(2))
        } else {
            Vec::with_capacity(2)
        };

        #[cfg(target_os = "macos")]
        search_dirs.push(PathBuf::from("/Library/Spelling/"));

        #[cfg(target_os = "linux")]
        search_dirs.push(PathBuf::from("/usr/share/myspell/"));

        Config {
            hunspell: Some(HunspellConfig {
                lang: Some("en_US".to_owned()),
                search_dirs: Some(search_dirs),
                extra_affixes: Some(Vec::new()),
                extra_dictonaries: Some(Vec::new()),
            }),
            languagetool: None,
        }
    }
}

// TODO figure out which ISO spec this actually is
pub struct CommonLang(String);

impl std::str::FromStr for CommonLang {
    type Err = anyhow::Error;
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
