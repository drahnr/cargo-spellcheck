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

#[cfg(test)]
mod tests;

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

    #[serde(alias = "ZSpell")]
    #[serde(default = "default_zspell")]
    pub zet: Option<ZetConfig>,

    #[serde(alias = "Spellbook")]
    #[serde(alias = "book")]
    #[serde(default = "default_spellbook")]
    pub spellbook: Option<SpellbookConfig>,

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
    const QUALIFIER: &'static str = "rs";
    const ORGANIZATION: &'static str = "fff";
    const APPLICATION: &'static str = "cargo_spellcheck";

    /// Sanitize all relative paths to absolute paths in relation to `base`.
    fn sanitize_paths(&mut self, base: &Path) -> Result<()> {
        let learning_targets = [
            self.hunspell.as_ref(),
            self.zet.as_ref(),
            self.spellbook.as_ref(),
        ]
        .into_iter()
        .flatten()
        .filter_map(|config| config.learning_dictionary.as_ref())
        .map(|path| base.join(path))
        .collect::<Vec<_>>();
        for config in [
            self.hunspell.as_mut(),
            self.zet.as_mut(),
            self.spellbook.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            config.sanitize_paths_with_learning_targets(base, &learning_targets)?;
        }
        Ok(())
    }

    fn spelling_backends(&self) -> impl Iterator<Item = &HunspellConfig> {
        [
            (cfg!(feature = "hunspell"), self.hunspell.as_ref()),
            (cfg!(feature = "zet"), self.zet.as_ref()),
            (cfg!(feature = "spellbook"), self.spellbook.as_ref()),
        ]
        .into_iter()
        .filter_map(|(compiled, config)| config.filter(|_| compiled))
    }

    /// Whether any configured spelling backend was compiled into this binary.
    pub fn has_spelling_backends(&self) -> bool {
        self.spelling_backends().next().is_some()
    }

    /// Resolve an existing, readable regular file without hiding filesystem errors.
    fn existing_learning_dictionary(path: &Path) -> Result<Option<PathBuf>> {
        match fs::symlink_metadata(path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        // Inspect the directory entry first so dangling symlinks are errors, not absence.
        let canonical = fs::canonicalize(path)?;
        if !fs::metadata(&canonical)?.is_file() {
            bail!(
                "Default learning dictionary {} must be a regular file",
                path.display()
            );
        }
        fs::File::open(&canonical)?;
        Ok(Some(canonical))
    }

    /// Select the project learning dictionary without creating any directories or files.
    /// The legacy location is used only when the preferred location is absent.
    pub fn default_learning_dictionary(base: &Path) -> Result<PathBuf> {
        let base = if base.is_absolute() {
            base.to_owned()
        } else {
            std::env::current_dir()?.join(base)
        };
        let preferred = base.join(".config/spellcheck/learnings.dic");
        if let Some(path) = Self::existing_learning_dictionary(&preferred)? {
            return Ok(path);
        }
        let legacy = base.join(".config/learnings.dic");
        if let Some(path) = Self::existing_learning_dictionary(&legacy)? {
            return Ok(path);
        }
        Ok(preferred)
    }

    /// Load an existing default for normal checks without validating explicit learning settings.
    /// Neither the TOML learning target nor the filesystem is modified.
    pub fn include_default_learning_dictionary(&mut self, base: &Path) -> Result<Option<PathBuf>> {
        if !self.has_spelling_backends()
            || self
                .spelling_backends()
                .any(|config| config.learning_dictionary.is_some())
        {
            return Ok(None);
        }
        let path = Self::default_learning_dictionary(base)?;
        if let Some(path) = Self::existing_learning_dictionary(&path)? {
            self.add_learning_dictionary(&path);
            Ok(Some(path))
        } else {
            Ok(None)
        }
    }

    /// Register an already validated absolute target for the next backend reload.
    /// This does not set an explicit learning target or write to the filesystem.
    pub fn add_learning_dictionary(&mut self, path: &Path) {
        for config in [
            (cfg!(feature = "hunspell"), self.hunspell.as_mut()),
            (cfg!(feature = "zet"), self.zet.as_mut()),
            (cfg!(feature = "spellbook"), self.spellbook.as_mut()),
        ]
        .into_iter()
        .filter_map(|(compiled, config)| config.filter(|_| compiled))
        {
            if !config.extra_dictionaries.iter().any(|extra| extra == path) {
                config.extra_dictionaries.push(path.to_owned());
            }
        }
    }

    /// Canonicalize the existing ancestor while preserving any missing path components.
    fn resolve_dictionary_path(path: &Path) -> Result<PathBuf> {
        let mut ancestor = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        let mut missing = Vec::new();
        loop {
            match fs::symlink_metadata(&ancestor) {
                Ok(_) => {
                    // A dangling symlink (including an ancestor) must not count as absence.
                    let mut resolved = fs::canonicalize(&ancestor)?;
                    if !missing.is_empty() && !fs::metadata(&resolved)?.is_dir() {
                        bail!(
                            "Dictionary ancestor {} must be a directory",
                            ancestor.display()
                        );
                    }
                    for component in missing.into_iter().rev() {
                        resolved.push(component);
                    }
                    return Ok(resolved);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let component = ancestor
                        .components()
                        .next_back()
                        .ok_or_else(|| eyre!("Cannot resolve dictionary {}", path.display()))?
                        .as_os_str()
                        .to_owned();
                    missing.push(component);
                    if !ancestor.pop() {
                        return Err(error.into());
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// Resolve a fix-time target and require a writable regular file if it exists.
    /// The returned flag is true when the target is absent; nothing is created or truncated.
    pub(crate) fn validate_learning_dictionary(path: &Path) -> Result<(PathBuf, bool)> {
        let resolved = Self::resolve_dictionary_path(path)?;
        match fs::symlink_metadata(&resolved) {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.permissions().readonly() {
                    bail!(
                        "Learning dictionary {} must be a writable regular file",
                        path.display()
                    );
                }
                fs::OpenOptions::new().write(true).open(&resolved)?;
                Ok((resolved, false))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((resolved, true)),
            Err(error) => Err(error.into()),
        }
    }

    /// Omit an absent target from backend inputs until learning creates and registers it.
    /// This only changes the in-memory extra dictionaries, never the configured target or TOML.
    pub(crate) fn omit_missing_learning_dictionary(&mut self, path: &Path) -> Result<()> {
        let target = Self::resolve_dictionary_path(path)?;
        match fs::symlink_metadata(&target) {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        for config in [
            (cfg!(feature = "hunspell"), self.hunspell.as_mut()),
            (cfg!(feature = "zet"), self.zet.as_mut()),
            (cfg!(feature = "spellbook"), self.spellbook.as_mut()),
        ]
        .into_iter()
        .filter_map(|(compiled, config)| config.filter(|_| compiled))
        {
            let extras = config
                .extra_dictionaries
                .iter()
                .map(|extra| Ok((extra, Self::resolve_dictionary_path(extra)?)))
                .collect::<Result<Vec<_>>>()?;
            config.extra_dictionaries = extras
                .into_iter()
                .filter(|(_, resolved)| resolved != &target)
                .map(|(extra, _)| extra.clone())
                .collect();
        }
        Ok(())
    }

    /// Validate and resolve one shared learning target after filtering enabled spelling engines.
    pub(crate) fn learning_dictionary(&self) -> Result<Option<PathBuf>> {
        let active = [
            (
                cfg!(feature = "hunspell"),
                "Hunspell",
                self.hunspell.as_ref(),
            ),
            (cfg!(feature = "zet"), "ZSpell", self.zet.as_ref()),
            (
                cfg!(feature = "spellbook"),
                "Spellbook",
                self.spellbook.as_ref(),
            ),
        ];
        let active = active
            .into_iter()
            .filter(|(compiled, _, _)| *compiled)
            .filter_map(|(_, name, config)| config.map(|config| (name, config)))
            .collect::<Vec<_>>();
        let mut target: Option<PathBuf> = None;
        let mut language = None;
        for (name, config) in &active {
            if language.is_some_and(|language| language != config.lang()) {
                bail!("Interactive vocabulary learning requires enabled spelling backends to use the same language ({name} differs)");
            }
            language = Some(config.lang());
            if let Some(path) = config.learning_dictionary.as_ref() {
                let (path, _) = Self::validate_learning_dictionary(path)
                    .wrap_err_with(|| format!("{name} learning_dictionary {} must be a writable regular file or nonexistent", path.display()))?;
                if target.as_ref().is_some_and(|target| target != &path) {
                    bail!("Conflicting learning_dictionary targets: all enabled spelling backends must share one dictionary");
                }
                target = Some(path);
            }
        }
        if let Some(target) = target.as_ref() {
            for (name, config) in active {
                let extra_dictionaries = config
                    .extra_dictionaries
                    .iter()
                    .map(|path| {
                        (|| -> Result<PathBuf> {
                            let resolved = Self::resolve_dictionary_path(path)?;
                            if &resolved != target && !fs::metadata(&resolved)?.is_file() {
                                bail!("Dictionary {} must be a regular file", path.display());
                            }
                            Ok(resolved)
                        })()
                        .wrap_err_with(|| format!("{name} extra_dictionaries {} must be an existing dictionary file or the learning target", path.display()))
                    })
                    .collect::<Result<Vec<_>>>()?;
                if !extra_dictionaries.contains(target) {
                    bail!("Learning dictionary {} must be listed in extra_dictionaries for every enabled spelling backend ({name} is missing it)", target.display());
                }
            }
        }
        Ok(target)
    }

    pub fn parse<S: AsRef<str>>(s: S) -> Result<Self> {
        Ok(toml::from_str(s.as_ref())?)
    }

    pub fn load_from<P: AsRef<Path>>(path: P) -> Result<Option<Self>> {
        let (contents, path) = match Self::load_content(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(e) => {
                bail!(e);
            }
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
            bail!("No idea where your config directory is located. XDG compliance would be nice.");
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
            bail!("No idea where your config directory is located. `$HOME` must be set.");
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
            );
        }
    }

    pub fn write_default_values() -> Result<Self> {
        let d = Self::default_path()?;
        Self::write_default_values_to(d.join("config.toml"))
    }

    pub fn is_enabled(&self, detector: Detector) -> bool {
        match detector {
            Detector::Hunspell => self.hunspell.is_some(),
            Detector::ZSpell => self.zet.is_some(),
            Detector::Spellbook => self.spellbook.is_some(),
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
fn default_zspell() -> Option<ZetConfig> {
    Some(ZetConfig::default())
}
fn default_spellbook() -> Option<SpellbookConfig> {
    Some(SpellbookConfig::default())
}

impl Default for Config {
    fn default() -> Self {
        Self {
            dev_comments: false,
            skip_readme: false,
            hunspell: default_hunspell(),
            zet: default_zspell(),
            spellbook: default_spellbook(),
            nlprules: default_nlprules(),
            reflow: Some(ReflowConfig::default()),
        }
    }
}

#[cfg(test)]
mod learning_tests;
