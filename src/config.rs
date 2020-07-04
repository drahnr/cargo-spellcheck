//! Configure cargo-spellcheck
//!
//! Supports `Hunspell` and `LanguageTool` scopes.
//!
//! A default configuration will be generated in the default
//! location by default. Default. Default default default.

use crate::suggestion::Detector;
use anyhow::{anyhow, Error, Result};
use log::trace;
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
    // must be option so it can be omitted in the config
    pub search_dirs: Option<Vec<PathBuf>>,
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
        if let Some(ref search_dirs) = &self.search_dirs {
            search_dirs.as_slice()
        } else {
            lazy_static::lazy_static! {
                static ref LOOKUP_DIRS: Vec<PathBuf> = vec![PathBuf::from("/usr/share/myspell")];
            };

            LOOKUP_DIRS.as_slice()
        }
    }

    pub fn extra_dictonaries(&self) -> &[PathBuf] {
        if let Some(ref extra_dictonaries) = self.extra_dictonaries {
            extra_dictonaries.as_slice()
        } else {
            &[]
        }
    }

    pub fn sanitize_paths(&mut self, base: &Path) -> Result<()> {
        if let Some(ref mut search_dirs) = &mut self.search_dirs {
            for path in search_dirs.iter_mut() {
                let abspath = if !path.is_absolute() {
                    base.join(path.clone())
                } else {
                    path.to_owned()
                };
                let abspath = std::fs::canonicalize(abspath)?;
                trace!(
                    "Sanitized ({} + {}) -> {}",
                    base.display(),
                    path.display(),
                    abspath.display()
                );
                *path = abspath;
            }
        }
        Ok(())
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
        let cfg =
            toml::from_str(s.as_ref()).map_err(|e| anyhow!("Failed parse toml").context(e))?;
        Ok(cfg)
    }

    pub fn load_from<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut file = File::open(path.as_ref().to_str().unwrap())
            .map_err(|e| anyhow!("Failed to open file {}", path.as_ref().display()).context(e))?;
        let mut contents = String::with_capacity(1024);
        file.read_to_string(&mut contents).map_err(|e| {
            anyhow!("Failed to read from file {}", path.as_ref().display()).context(e)
        })?;
        Self::parse(&contents).and_then(|mut cfg| {
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
            Err(anyhow!(
                "No idea where your config directory is located. XDG compliance would be nice."
            ))
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
            Err(anyhow!(
                "No idea where your config directory is located. `$HOME` must be set."
            ))
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
        search_dirs.extend(vec![
            // Fedora
            PathBuf::from("/usr/share/myspell/"),
            // Arch Linux
            PathBuf::from("/usr/share/hunspell/"),
            PathBuf::from("/usr/share/myspell/dicts/"),
        ]);

        Self {
            hunspell: Some(HunspellConfig {
                lang: Some("en_US".to_owned()),
                search_dirs: Some(search_dirs),
                extra_dictonaries: Some(Vec::new()),
            }),
            languagetool: None,
        }
    }
}

// @todo figure out which ISO spec this actually is
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
url = "127.0.0.1:8010/"

[Hunspell]
lang = "en_US"
search_dirs = ["/usr/lib64/hunspell"]
extra_dictonaries = ["/home/bernhard/test.dic"]
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
