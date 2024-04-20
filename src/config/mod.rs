//! Configure cargo-spellcheck
//!
//! Supports `Hunspell` and `LanguageTool` scopes.
//!
//! A default configuration will be generated in the default location by
//! default. Default. Default default default.

// TODO pendeng refactor, avoid spending time on documenting the status quo.
#![allow(missing_docs)]

pub mod args;

mod regex;
pub use self::regex::*;

mod reflow;
pub use self::reflow::*;

mod hunspell;
pub use self::hunspell::*;

mod nlprules;
pub use self::nlprules::*;

mod search_dirs;
pub use search_dirs::*;

mod iso;
pub use iso::*;

use crate::errors::*;
use crate::Detector;
use fancy_regex::Regex;

use fs_err as fs;
use serde::{Deserialize, Serialize};
use std::convert::AsRef;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Config {
    // Options that modify the inputs being picked up.
    #[serde(default)]
    #[serde(alias = "dev-comments")]
    #[serde(alias = "devcomments")]
    pub dev_comments: bool,

    #[serde(default)]
    #[serde(alias = "skip-readme")]
    #[serde(alias = "skipreadme")]
    pub skip_readme: bool,

    #[serde(alias = "Hunspell")]
    #[serde(default = "default_hunspell")]
    pub hunspell: Option<HunspellConfig>,

    #[serde(alias = "Nlp")]
    #[serde(alias = "NLP")]
    #[serde(alias = "nlp")]
    #[serde(alias = "NLP")]
    #[serde(alias = "NlpRules")]
    #[serde(default = "default_nlprules")]
    pub nlprules: Option<NlpRulesConfig>,

    #[serde(alias = "ReFlow")]
    #[serde(alias = "Reflow")]
    pub reflow: Option<ReflowConfig>,
}

impl Config {
    const QUALIFIER: &'static str = "io";
    const ORGANIZATION: &'static str = "spearow";
    const APPLICATION: &'static str = "cargo_spellcheck";

    /// Sanitize all relative paths to absolute paths in relation to `base`.
    fn sanitize_paths(&mut self, base: &Path) -> Result<()> {
        if let Some(ref mut hunspell) = self.hunspell {
            hunspell.sanitize_paths(base)?;
        }
        Ok(())
    }

    pub fn parse<S: AsRef<str>>(s: S) -> Result<Self> {
        Ok(toml::from_str(s.as_ref())?)
    }

    pub fn load_from<P: AsRef<Path>>(path: P) -> Result<Option<Self>> {
        let (contents, path) = match Self::load_content(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(e) => bail!(e),
            Ok(contents) => contents,
        };
        Self::parse(&contents)
            .wrap_err_with(|| {
                eyre!(
                    "Syntax of a given config file({}) is broken",
                    path.display()
                )
            })
            .and_then(|mut cfg| {
                if let Some(base) = path.parent() {
                    cfg.sanitize_paths(base)?;
                }
                Ok(Some(cfg))
            })
    }

    pub fn load_content<P: AsRef<Path>>(path: P) -> std::io::Result<(String, PathBuf)> {
        let path = path.as_ref().canonicalize()?;
        let mut file = fs::File::open(&path)?;

        let mut contents = String::with_capacity(1024);
        file.read_to_string(&mut contents)?;
        Ok((contents, path))
    }

    pub fn load() -> Result<Option<Self>> {
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
        toml::to_string(self).wrap_err_with(|| eyre!("Failed to convert to toml"))
    }

    pub fn write_values_to<W: std::io::Write>(&self, mut writer: W) -> Result<Self> {
        let s = self.to_toml()?;
        writer.write_all(s.as_bytes())?;
        Ok(self.clone())
    }

    pub fn write_values_to_path<P: AsRef<Path>>(&self, path: P) -> Result<Self> {
        let path = path.as_ref();

        if let Some(path) = path.parent() {
            fs::create_dir_all(path).wrap_err_with(|| {
                eyre!("Failed to create config parent dirs {}", path.display())
            })?;
        }

        let file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .wrap_err_with(|| eyre!("Failed to write default values to {}", path.display()))?;

        let writer = std::io::BufWriter::new(file);

        self.write_values_to(writer)
            .wrap_err_with(|| eyre!("Failed to write default config to {}", path.display()))
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
            Detector::NlpRules => self.nlprules.is_some(),
            Detector::Reflow => self.reflow.is_some(),
            #[cfg(test)]
            Detector::Dummy => true,
        }
    }

    pub fn full() -> Self {
        Default::default()
    }
}

fn default_nlprules() -> Option<NlpRulesConfig> {
    if cfg!(feature = "nlprules") {
        Some(NlpRulesConfig::default())
    } else {
        log::warn!("Cannot enable nlprules, since it wasn't compiled with `nlprules` as checker");
        None
    }
}

fn default_hunspell() -> Option<HunspellConfig> {
    Some(HunspellConfig::default())
}

impl Default for Config {
    fn default() -> Self {
        Self {
            dev_comments: false,
            skip_readme: false,
            hunspell: default_hunspell(),
            nlprules: default_nlprules(),
            reflow: Some(ReflowConfig::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;

    #[test]
    fn can_serialize_to_toml() {
        let config = dbg!(Config::full());
        assert_matches!(config.to_toml(), Ok(_s));
    }

    #[test]
    fn project_config_works() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".config")
            .join("spellcheck.toml");
        assert_matches!(Config::load_from(&path), Ok(_));
    }

    #[test]
    fn all() {
        let _ = Config::parse(
            r#"
dev_comments = true
skip-readme = true

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
lang = "en_GB"
search_dirs = ["/usr/lib64/hunspell"]
extra_dictionaries = ["/home/bernhard/test.dic"]
			"#,
        )
        .unwrap();
    }

    #[test]
    fn partial_3() {
        let cfg = Config::parse(
            r#"
[Hunspell]
lang = "de_AT"
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
[Hunspell]
lang = "en_US"
			"#,
        )
        .unwrap();
        let _hunspell = cfg.hunspell.expect("Must contain hunspell cfg");
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
skip_os_lookups = true
			"#,
        )
        .unwrap();

        let hunspell: HunspellConfig = cfg.hunspell.expect("Must contain hunspell cfg");
        assert!(hunspell.skip_os_lookups);

        let search_dirs = hunspell.search_dirs;
        let search_dirs2: Vec<_> = search_dirs.as_ref().clone();
        assert!(!search_dirs2.is_empty());

        assert_eq!(search_dirs.iter(false).count(), 2);

        #[cfg(target_os = "linux")]
        assert_eq!(search_dirs.iter(true).count(), 5);

        #[cfg(target_os = "windows")]
        assert_eq!(search_dirs.iter(true).count(), 2);

        #[cfg(target_os = "macos")]
        assert!(search_dirs.iter(true).count() >= 3);
    }

    #[test]
    fn partial_9() {
        let cfg = Config::parse(
            r#"
[Reflow]
max_line_length = 42
"#,
        )
        .unwrap();
        assert_eq!(
            cfg.reflow.expect("Must contain reflow cfg").max_line_length,
            42
        );
    }
}
