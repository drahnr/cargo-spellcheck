use std::path::{Path, PathBuf};

use crate::errors::*;

use fs_err as fs;
use serde::Deserialize;
use std::str::FromStr;

use crate::Action;

use super::Config;

use clap_complete::Shell;

#[derive(Debug, Clone, Hash, PartialEq, Eq, Deserialize)]
pub struct ManifestMetadata {
    spellcheck: Option<ManifestMetadataSpellcheck>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Deserialize)]
pub struct ManifestMetadataSpellcheck {
    config: PathBuf,
}

/// Checker types to be derived from the stringly typed arguments.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Deserialize)]
pub enum CheckerType {
    Hunspell,
    ZSpell,
    Spellbook,
    NlpRules,
    Reflow,
}

impl FromStr for CheckerType {
    type Err = UnknownCheckerTypeVariant;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_lowercase();
        Ok(match s.as_str() {
            "nlprules" => Self::NlpRules,
            "zet" | "zspell" => Self::ZSpell,
            "spellbook" | "book" => Self::Spellbook,
            "hunspell" => Self::Hunspell,
            "reflow" => Self::Reflow,
            _other => return Err(UnknownCheckerTypeVariant(s)),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MultipleCheckerTypes(pub Vec<CheckerType>);

impl AsRef<[CheckerType]> for MultipleCheckerTypes {
    fn as_ref(&self) -> &[CheckerType] {
        self.0.as_slice()
    }
}

impl std::ops::Deref for MultipleCheckerTypes {
    type Target = [CheckerType];
    fn deref(&self) -> &Self::Target {
        self.0.as_slice()
    }
}

impl IntoIterator for MultipleCheckerTypes {
    type Item = CheckerType;
    type IntoIter = <Vec<Self::Item> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromStr for MultipleCheckerTypes {
    type Err = UnknownCheckerTypeVariant;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.split(',')
            .map(<CheckerType as FromStr>::from_str)
            .collect::<Result<Vec<_>, _>>()
            .map(MultipleCheckerTypes)
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("Unknown checker type variant: {0}")]
pub struct UnknownCheckerTypeVariant(String);

#[derive(clap::Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(rename_all = "kebab-case")]
#[clap(subcommand_negates_reqs(true))]
pub struct Args {
    /// Print full source paths instead of paths relative to the input directory or cwd.
    #[clap(long, global(true))]
    pub full_paths: bool,

    #[clap(short, long, global(true))]
    /// Provide a configuration.
    pub cfg: Option<PathBuf>,

    #[clap(flatten)]
    pub verbosity: clap_verbosity_flag::Verbosity,

    // is required, but we use `subcommand_negates_reqs`, so it's not
    // when a command exists
    #[clap(flatten)]
    /// Short-cut for `cargo spellcheck check`.
    pub common: Common,

    #[clap(short, long)]
    /// Alt for `cargo spellcheck fix` [deprecated].
    pub fix: bool,

    #[clap(subcommand)]
    /// Available sub-commands.
    pub command: Option<Sub>,
}

#[derive(Debug, PartialEq, Eq, clap::Parser)]
#[clap(rename_all = "kebab-case")]
pub struct Common {
    #[clap(short, long)]
    /// Recurse based on the current directory, or all given
    /// argument paths, and also declared modules in rust files.
    pub recursive: bool,

    // with fallback from config, so it has to be tri-state
    #[clap(long)]
    /// Execute the given subset of checkers.
    pub checkers: Option<MultipleCheckerTypes>,

    #[clap(short, long)]
    /// Do not check the referenced key `readme=` or default `README.md`.
    pub skip_readme: bool,

    #[clap(short, long)]
    /// Also check developer comments besides documentation comments.
    pub dev_comments: bool,

    #[clap(short, long)]
    /// The number of worker threads to spawn for the actual processing text.
    pub jobs: Option<usize>,

    #[clap(short = 'm', long, default_value_t = 0_u8)]
    /// Return code of the application if spelling mistakes were found.
    pub code: u8,

    /// A list of files and directories to check. See `--recursive`.
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, PartialEq, Eq, clap::Subcommand)]
#[clap(rename_all = "kebab-case")]
pub enum Sub {
    /// Only show check errors, but do not request user input.
    // `cargo spellcheck` is short for checking.
    Check {
        #[clap(flatten)]
        common: Common,
    },

    /// Interactively choose from checker provided suggestions.
    Fix {
        #[clap(flatten)]
        common: Common,
    },

    /// Reflow doc comments, so they adhere to a given maximum column width.
    Reflow {
        #[clap(flatten)]
        common: Common,
    },

    /// Print the config being in use, default config if none.
    Config {
        #[clap(short, long)]
        /// Write to the default user configuration file path.
        user: bool,

        #[clap(short, long)]
        /// Force overwrite an existing user config.
        overwrite: bool,

        #[clap(short, long)]
        /// Write to `stdout`.
        stdout: bool,

        #[clap(long)]
        // Deprecated alias, will be removed in the future.
        #[clap(alias = "checkers")]
        /// Limit checkers to enable in the generated configuration.
        filter: Option<MultipleCheckerTypes>,
    },

    /// List all files in depth-first-sorted-order in which they would be
    /// checked.
    ListFiles {
        #[clap(short, long)]
        /// Recurse down directories and module declaration derived paths.
        recursive: bool,

        #[clap(short, long)]
        /// Do not check the referenced key `readme=` or default `README.md`.
        skip_readme: bool,

        /// A list of files and directories to check. See `--recursive`.
        paths: Vec<PathBuf>,
    },

    /// Print completions.
    Completions {
        #[clap(long, env="SHELL", value_parser = load_shell_name)]
        /// Provide the `shell` for which to generate the completion script.
        shell: Shell,
    },
}

#[derive(thiserror::Error, Debug, Clone)]
enum ShellErr {
    #[error("Unknown shell: {shell:?}")]
    UnknownShell { shell: String },

    #[error("Missing SHELL argument")]
    MissingArg,
}

/// Either provided as a path with `/` as separator or directly.
fn load_shell_name(shell: &str) -> Result<Shell, ShellErr> {
    shell
        .split('/')
        .next_back()
        .map(|shell| {
            Shell::from_str(shell).map_err(|unknown_shell| ShellErr::UnknownShell {
                shell: unknown_shell,
            })
        })
        .unwrap_or_else(|| Err(ShellErr::MissingArg))
}

pub fn generate_completions<G: clap_complete::Generator, W: std::io::Write>(
    generator: G,
    sink: &mut W,
) {
    let mut app = <Args as clap::CommandFactory>::command();
    let app = &mut app;
    clap_complete::generate(generator, app, app.get_name().to_string(), sink);
}

impl Args {
    pub fn common(&self) -> Option<&Common> {
        match &self.command {
            Some(
                Sub::Check { common, .. } | Sub::Fix { common, .. } | Sub::Reflow { common, .. },
            ) => Some(common),
            None => Some(&self.common),
            Some(Sub::Completions { .. } | Sub::ListFiles { .. } | Sub::Config { .. }) => None,
        }
    }

    pub fn checkers(&self) -> Option<Vec<CheckerType>> {
        self.common()
            .and_then(|common| common.checkers.as_ref().map(|checkers| checkers.0.clone()))
    }

    pub fn job_count(&self) -> usize {
        derive_job_count(self.common().and_then(|common| common.jobs))
    }

    /// Extract the verbosity level
    pub fn verbosity(&self) -> log::LevelFilter {
        self.verbosity.log_level_filter()
    }

    /// Extract the required action.
    pub fn action(&self) -> Action {
        // extract operation mode
        let action = if let Some(sub) = &self.command {
            match sub {
                Sub::Check { .. } => Action::Check,
                Sub::Fix { .. } => Action::Fix,
                Sub::Reflow { .. } => Action::Reflow,
                Sub::ListFiles { .. } => Action::ListFiles,
                Sub::Config { .. } => unreachable!(),
                Sub::Completions { .. } => unreachable!(),
            }
        } else if self.fix {
            Action::Fix
        } else {
            Action::Check
        };
        log::trace!("Derived action {action:?} from flags/args/cmds");
        action
    }

    /// Adjust the raw arguments for call variants.
    ///
    /// The program could be called like `cargo-spellcheck`, `cargo spellcheck`
    /// or `cargo spellcheck check` and even ``cargo-spellcheck check`.
    pub fn parse(argv_iter: impl IntoIterator<Item = String>) -> Result<Self, clap::Error> {
        <Args as clap::Parser>::try_parse_from({
            // if ends with file name `cargo-spellcheck`
            let mut argv_iter = argv_iter.into_iter();
            if let Some(arg0) = argv_iter.next() {
                match PathBuf::from(&arg0).file_name().and_then(|x| x.to_str()) {
                    Some(file_name) => {
                        // allow all variants to be parsed
                        // cargo spellcheck ...
                        // cargo-spellcheck ...
                        // cargo-spellcheck spellcheck ...
                        //
                        // so preprocess them to unified `cargo-spellcheck`
                        let mut next = vec!["cargo-spellcheck".to_owned()];

                        match argv_iter.next() {
                            Some(arg)
                                if file_name.starts_with("cargo-spellcheck")
                                    && arg == "spellcheck" =>
                            {
                                // drop the first arg `spellcheck`
                            }
                            Some(arg) if file_name.starts_with("cargo") && &arg == "spellcheck" => {
                                // drop it, we replace it with `cargo-spellcheck`
                            }
                            Some(arg) if arg == "spellcheck" => {
                                // "spellcheck" but the binary got renamed
                                // drop the "spellcheck" part
                            }
                            Some(arg) => {
                                // not "spellcheck" so retain it
                                next.push(arg.to_owned())
                            }
                            None => {}
                        };
                        Vec::from_iter(next.into_iter().chain(argv_iter))
                    }
                    _ => Vec::from_iter(argv_iter),
                }
            } else {
                Vec::new()
            }
        })
    }

    /// Overrides the enablement status of checkers in the configuration based
    /// on the checkers enabled by argument, if it is set.
    ///
    /// Errors if no compiled checker is left. Reflow is configured separately.
    pub fn checker_selection_override(
        filter_set: Option<&[CheckerType]>,
        config: &mut Config,
    ) -> Result<()> {
        // overwrite checkers
        if let Some(checkers) = filter_set {
            if (!cfg!(feature = "hunspell") || !checkers.contains(&CheckerType::Hunspell))
                && config.hunspell.take().is_none()
            {
                log::warn!("Hunspell was never configured.")
            }
            if (!cfg!(feature = "nlprules") || !checkers.contains(&CheckerType::NlpRules))
                && config.nlprules.take().is_none()
            {
                log::warn!("Nlprules checker was never configured.")
            }

            if (!cfg!(feature = "zet") || !checkers.contains(&CheckerType::ZSpell))
                && config.zet.take().is_none()
            {
                log::warn!("ZSpell was never configured.")
            }
            if (!cfg!(feature = "spellbook") || !checkers.contains(&CheckerType::Spellbook))
                && config.spellbook.take().is_none()
            {
                log::warn!("Spellbook was never configured.")
            }

            if !checkers.contains(&CheckerType::Reflow) {
                log::warn!("Reflow is a separate sub command.")
            }

            if !config.has_spelling_backends()
                && !(cfg!(feature = "nlprules") && config.nlprules.is_some())
            {
                bail!("Argument override for checkers disabled all checkers");
            }
        }
        Ok(())
    }

    /// Load configuration with fallbacks.
    ///
    /// Does IO checks if files exist.
    ///
    /// Provides a config and where it was retrieved from, if no config file
    /// exists, a default is provided and the config path becomes `None`.
    ///
    /// 1. explicitly specified cli flag, error if it does not exist or parse
    /// 2. `Cargo.toml` metadata (unimplemented), error if it does not exist or parse
    /// 3. find a `Cargo.toml` and try to find `.config/spellcheck.toml` error if it does not parse
    /// 4. Fallback to per-user config, error if it does not parse
    /// 5. Default config, error if it does not parse
    ///
    // TODO split the IO operations and lookup dirs.
    fn load_config_inner(&self) -> Result<(Config, Option<PathBuf>)> {
        log::debug!("Attempting to load configuration by priority.");
        let cwd = crate::traverse::cwd()?;
        // 1. explicitly specified
        let explicit_cfg = self.cfg.as_ref().map(|config_path| {
            if config_path.is_absolute() {
                config_path.to_owned()
            } else {
                // TODO make sure this is sane behavior
                // to use `cwd`.
                cwd.join(config_path)
            }
        });

        if let Some(config_path) = explicit_cfg {
            log::debug!(
                "Using configuration file provided by flag (1) {}",
                config_path.display()
            );
            let config =
                Config::load_from(&config_path)?.ok_or_else(|| eyre!("File does not exist."))?;
            return Ok((config, Some(config_path)));
        }

        log::debug!("No cfg flag present");

        // (prep) determine if there should be an attempt to read a cargo manifest from the target dir
        let single_target_path = self.common().and_then(|common| {
            common
                .paths
                .first()
                .filter(|_x| common.paths.len() == 1)
                .cloned()
        });

        // 2. manifest meta in target dir
        let manifest_path_in_target_dir = if let Some(ref base) = single_target_path {
            look_for_cargo_manifest(base)?
        } else {
            None
        };
        if let Some(manifest_path) = &manifest_path_in_target_dir {
            if let Some((config, config_path)) = load_from_manifest_metadata(manifest_path)? {
                return Ok((config, Some(config_path)));
            }
        };

        // 3. manifest meta in current working dir
        if let Some(manifest_path) = look_for_cargo_manifest(&cwd)? {
            if let Some((config, config_path)) = load_from_manifest_metadata(&manifest_path)? {
                return Ok((config, Some(config_path)));
            }
        };

        // 4. load from `.config/spellcheck.toml` from the current working directory.
        let config_path = cwd.join(".config").join("spellcheck.toml");
        if let Some(cfg) = Config::load_from(&config_path)? {
            log::debug!("Using configuration file (4) {}", config_path.display());
            return Ok((cfg, Some(config_path)));
        }

        let default_config_path = Config::default_path()?;
        if let Some(cfg) = Config::load_from(&default_config_path)? {
            log::debug!(
                "Using configuration file (5) {}",
                default_config_path.display()
            );
            return Ok((cfg, Some(default_config_path)));
        }

        log::debug!("No user config present {}", default_config_path.display());
        log::debug!("Using configuration default, builtin configuration (5)");
        Ok((Config::default(), None))
    }

    fn load_config(&self) -> Result<(Config, Option<PathBuf>)> {
        let (mut config, config_path) = self.load_config_inner()?;
        // mask all disabled checkers, use the default config
        // for those which have one if not enabled already.

        // FIXME: Due to an increase adoption, having `NlpRules` enabled by default,
        // causes friction for users, especially in presence of inline codes which are
        // elided, and cause even worse suggestions.
        // ISSUE: https://github.com/drahnr/cargo-spellcheck/issues/242
        let filter_set = self.checkers().unwrap_or_else(|| {
            [
                (cfg!(feature = "hunspell"), CheckerType::Hunspell),
                (cfg!(feature = "spellbook"), CheckerType::Spellbook),
                (cfg!(feature = "zet"), CheckerType::ZSpell),
            ]
            .into_iter()
            .find_map(|(compiled, checker)| compiled.then_some(checker))
            .into_iter()
            .collect()
        });
        {
            if cfg!(feature = "hunspell") && filter_set.contains(&CheckerType::Hunspell) {
                if config.hunspell.is_none() {
                    config.hunspell = Some(crate::config::HunspellConfig::default());
                }
            } else {
                config.hunspell = None;
            }
            if cfg!(feature = "zet") && filter_set.contains(&CheckerType::ZSpell) {
                if config.zet.is_none() {
                    config.zet = Some(crate::config::ZetConfig::default());
                }
            } else {
                config.zet = None;
            }
            if cfg!(feature = "spellbook") && filter_set.contains(&CheckerType::Spellbook) {
                if config.spellbook.is_none() {
                    config.spellbook = Some(crate::config::SpellbookConfig::default());
                }
            } else {
                config.spellbook = None;
            }
            if cfg!(feature = "nlprules") && filter_set.contains(&CheckerType::NlpRules) {
                if config.nlprules.is_none() {
                    config.nlprules = Some(crate::config::NlpRulesConfig::default());
                }
            } else {
                config.nlprules = None;
            }
            // reflow is a different subcommand, not relevant
        }

        if matches!(
            &self.command,
            None | Some(Sub::Check { .. } | Sub::Fix { .. })
        ) && !config.has_spelling_backends()
            && !(cfg!(feature = "nlprules") && config.nlprules.is_some())
        {
            bail!("No compiled checker is enabled; select an available checker with --checkers");
        }

        Ok((config, config_path))
    }

    /// Evaluate the configuration flags, overwrite config values as needed and
    /// provide a new, unified config struct.
    pub fn unified(self) -> Result<(UnifiedArgs, Config)> {
        let (config, config_path) = self.load_config()?;
        let unified = match self.command {
            Some(Sub::Config {
                stdout,
                user,
                overwrite,
                filter: checkers,
            }) => {
                let dest_config = match self.cfg {
                    None if stdout => ConfigWriteDestination::Stdout,
                    Some(path) => ConfigWriteDestination::File { overwrite, path },
                    None if user => ConfigWriteDestination::File {
                        overwrite,
                        path: Config::default_path()?,
                    },
                    _ => {
                        bail!("Neither --user or --stdout are given, invalid flags passed.");
                    }
                };
                UnifiedArgs::Config {
                    dest_config,
                    checker_filter_set: checkers,
                }
            }
            Some(Sub::ListFiles {
                ref paths,
                recursive,
                skip_readme,
            }) => UnifiedArgs::Operate {
                action: self.action(),
                config_path,
                dev_comments: false, // not relevant
                skip_readme,
                recursive,
                paths: paths.clone(),
                exit_code_override: 1,
            },
            None => {
                let common = &self.common;
                UnifiedArgs::Operate {
                    action: Action::Check,
                    config_path,
                    dev_comments: common.dev_comments || config.dev_comments,
                    skip_readme: common.skip_readme || config.skip_readme,
                    recursive: common.recursive,
                    paths: common.paths.clone(),
                    exit_code_override: common.code,
                }
            }
            Some(
                Sub::Reflow { ref common, .. }
                | Sub::Fix { ref common, .. }
                | Sub::Check { ref common, .. },
            ) => UnifiedArgs::Operate {
                action: self.action(),
                config_path,
                dev_comments: common.dev_comments || config.dev_comments,
                skip_readme: common.skip_readme || config.skip_readme,
                recursive: common.recursive,
                paths: common.paths.clone(),
                exit_code_override: common.code,
            },
            Some(Sub::Completions { .. }) => unreachable!("Was handled earlier. qed"),
        };

        Ok((unified, config))
    }
}

#[derive(Debug, Clone)]
pub enum ConfigWriteDestination {
    Stdout,
    File { overwrite: bool, path: PathBuf },
}

/// Unified arguments with configuration fallbacks.
///
/// Only contains options which are either only present in the arguments, or are
/// present in the arguments and have a fallback in the configuration.
#[derive(Debug, Clone)]
pub enum UnifiedArgs {
    Config {
        dest_config: ConfigWriteDestination,
        checker_filter_set: Option<MultipleCheckerTypes>,
    },
    Operate {
        action: Action,
        config_path: Option<PathBuf>,
        dev_comments: bool,
        skip_readme: bool,
        recursive: bool,
        paths: Vec<PathBuf>,
        exit_code_override: u8,
    },
}

impl UnifiedArgs {
    /// Extract the action.
    pub fn action(&self) -> Action {
        match self {
            Self::Operate { action, .. } => *action,
            _ => unreachable!(),
        }
    }
}

/// Try to find a cargo manifest, given a path, that can either be a directory
/// or a path to a manifest.
fn look_for_cargo_manifest(base: &Path) -> Result<Option<PathBuf>> {
    Ok(if base.is_dir() {
        let base = base.join("Cargo.toml");
        if base.is_file() {
            let base = base.canonicalize()?;
            log::debug!("Using {} manifest as anchor file", base.display());
            Some(base)
        } else {
            log::debug!("Cargo manifest files does not exist: {}", base.display());
            None
        }
    } else if let Some(file_name) = base.file_name() {
        if file_name == "Cargo.toml" && base.is_file() {
            let base = base.canonicalize()?;
            log::debug!("Using {} manifest as anchor file", base.display());
            Some(base)
        } else {
            log::debug!("Cargo manifest files does not exist: {}", base.display());
            None
        }
    } else {
        log::debug!(
            "Provided parse target is neither file or dir: {}",
            base.display()
        );
        None
    })
}

fn extract_config_path_from_metadata(
    manifest_path: &Path,
    metadata: ManifestMetadata,
    ident: &str,
) -> Result<Option<(Config, PathBuf)>> {
    if let Some(spellcheck) = metadata.spellcheck {
        let config_path = &spellcheck.config;
        let config_path = if config_path.is_absolute() {
            config_path.to_owned()
        } else {
            let manifest_dir = manifest_path.parent().expect("File resides in a dir. qed");
            manifest_dir.join(config_path)
        };
        log::debug!(
            "Using configuration ({}) file {}",
            ident,
            config_path.display()
        );
        return Ok(Config::load_from(&config_path)?.map(|config| (config, config_path)));
    }
    Ok(None)
}

fn load_from_manifest_metadata(manifest_path: &Path) -> Result<Option<(Config, PathBuf)>> {
    let manifest = fs::read_to_string(manifest_path)?;
    let manifest =
        cargo_toml::Manifest::<ManifestMetadata>::from_slice_with_metadata(manifest.as_bytes())
            .wrap_err(format!(
                "Failed to parse cargo manifest: {}",
                manifest_path.display()
            ))?;
    if let Some(metadata) = manifest.package.and_then(|package| package.metadata) {
        if let Some(x) = extract_config_path_from_metadata(manifest_path, metadata, "package")? {
            return Ok(Some(x));
        }
    }
    if let Some(metadata) = manifest.workspace.and_then(|workspace| workspace.metadata) {
        if let Some(x) = extract_config_path_from_metadata(manifest_path, metadata, "workspace")? {
            return Ok(Some(x));
        }
    }
    Ok(None)
}

/// Set the worker pool job/thread count.
///
/// Affects the parallel processing for a particular checker. Checkers are
/// always executed in sequence.
pub fn derive_job_count(jobs: impl Into<Option<usize>>) -> usize {
    let maybe_jobs = jobs.into();
    match maybe_jobs {
        _ if cfg!(debug_assertions) => {
            log::warn!("Debug mode always uses 1 thread!");
            1
        }
        Some(jobs) if jobs == 0 => {
            log::warn!(
                "Cannot have less than one worker thread ({jobs}). Retaining one worker thread."
            );
            1
        }
        Some(jobs) if jobs > 128 => {
            log::warn!("Setting threads beyond 128 ({jobs}) is insane. Capping at 128");
            128
        }
        Some(jobs) => {
            log::info!("Explicitly set threads to {jobs}");
            jobs
        }
        None => {
            // commonly we are not the only process
            // on the machine, so use the physical cores.
            let jobs = num_cpus::get_physical();
            log::debug!("Using the default physical thread count of {jobs}");
            jobs
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;

    fn commandline_to_iter(s: &'static str) -> impl Iterator<Item = String> {
        s.split(' ').map(|s| s.to_owned())
    }

    lazy_static::lazy_static!(
        static ref SAMPLES: std::collections::HashMap<&'static str, Action> = maplit::hashmap!{
            // check (implicit)
            "cargo spellcheck" => Action::Check,
            "cargo spellcheck -vvvv" => Action::Check,
            "cargo-spellcheck" => Action::Check,
            "cargo-spellcheck -vvvv" => Action::Check,
            // check (explicit)
            "cargo spellcheck check -m 11" => Action::Check,
            "cargo-spellcheck check -m 9" => Action::Check,
            // reflow
            "cargo spellcheck reflow" => Action::Reflow,
            "cargo-spellcheck reflow" => Action::Reflow,
            // fix (deprecated)
            "cargo spellcheck --fix" => Action::Fix,
            "cargo-spellcheck --fix" => Action::Fix,
            // fix
            "cargo spellcheck fix" => Action::Fix,
            "cargo-spellcheck fix" => Action::Fix,
            "cargo-spellcheck fix -r file.rs" => Action::Fix,
            "cargo-spellcheck -q fix Cargo.toml" => Action::Fix,
            "cargo spellcheck -v fix Cargo.toml" => Action::Fix,

            // FIXME check it fully, against the unified args
            // TODO must implement an abstraction for the config file source for that
            // "cargo spellcheck completions --shell zsh" => Sub::Completions { shell: Shell::Zsh },
            // "cargo spellcheck completions --shell bash" => Sub::Completions { shell: Shell::Bash },
            // "cargo-spellcheck completions --shell zsh" => Sub::Completions { shell: Shell::Zsh },
            // "cargo-spellcheck completions --shell bash" => Sub::Completions { shell: Shell::Bash },
            // "cargo-spellcheck completions" => Sub::Completions { .. },
        };
    );

    #[test]
    fn args() {
        for command in SAMPLES.keys() {
            assert_matches!(Args::parse(commandline_to_iter(command)), Ok(_));
        }
    }

    #[test]
    fn deserialize_multiple_checkers() {
        let args = Args::parse(commandline_to_iter(
            "cargo spellcheck check --checkers=nlprules,hunspell",
        ))
        .expect("Parsing works. qed");
        assert_eq!(
            args.checkers(),
            Some(vec![CheckerType::NlpRules, CheckerType::Hunspell])
        );
    }

    #[test]
    fn alt_fix_works() {
        let args_sub = Args::parse(commandline_to_iter("cargo spellcheck fix")).unwrap();
        let args_alt = Args::parse(commandline_to_iter("cargo spellcheck --fix")).unwrap();
        assert_eq!(args_sub.common(), args_alt.common());
        assert_eq!(args_sub.action(), args_alt.action());
    }

    #[test]
    fn unify_ops_check() {
        let args = Args::parse(
            [
                "cargo",
                "spellcheck",
                "-vvvvv",
                "check",
                "--code=77",
                "--dev-comments",
                "--skip-readme",
                "--checkers=hunspell,nlprules,zet,spellbook",
            ]
            .iter()
            .map(ToOwned::to_owned)
            .map(ToOwned::to_owned),
        )
        .unwrap();
        let (unified, _config) = args.unified().unwrap();
        assert_matches!(unified,
            UnifiedArgs::Operate {
                action,
                config_path: _,
                dev_comments,
                skip_readme,
                recursive,
                paths,
                exit_code_override,
            } => {
                assert_eq!(Action::Check, action);
                assert_eq!(exit_code_override, 77);
                assert_eq!(dev_comments, true);
                assert_eq!(skip_readme, true);
                assert_eq!(recursive, false);
                assert_eq!(paths, Vec::<PathBuf>::new());
            }
        );
    }

    // FIXME checkers interpretation seems to have changed XXX
    #[test]
    fn unify_config() {
        let args = Args::parse(
            [
                "cargo-spellcheck",
                "--cfg=.config/spellcheck.toml",
                "config",
                "--checkers=NlpRules",
                "--overwrite",
            ]
            .iter()
            .map(ToOwned::to_owned)
            .map(ToOwned::to_owned),
        )
        .unwrap();
        let (unified, _config) = dbg!(args).unified().unwrap();
        assert_matches!(dbg!(unified),
            UnifiedArgs::Config {
                dest_config: ConfigWriteDestination::File { overwrite, path },
                checker_filter_set,
            } => {
                assert_eq!(path, PathBuf::from(".config/spellcheck.toml"));
                assert_eq!(checker_filter_set, Some(MultipleCheckerTypes(vec![CheckerType::NlpRules])));
                assert_eq!(overwrite, true);
            }
        );
    }

    fn load_test_config(command: &'static str) -> Result<Config> {
        let mut args = Args::parse(commandline_to_iter(command))?;
        let path = std::env::temp_dir().join(format!("spellcheck-{}.toml", uuid::Uuid::new_v4()));
        fs::write(&path, "")?;
        args.cfg = Some(path.clone());
        let result = args.load_config().map(|(config, _)| config);
        fs::remove_file(path)?;
        result
    }

    #[test]
    fn default_checker_is_a_compiled_spelling_backend() {
        use crate::checker::{Checker, Checkers};

        let result = load_test_config("cargo spellcheck");
        if !cfg!(any(
            feature = "hunspell",
            feature = "spellbook",
            feature = "zet"
        )) {
            assert!(result.is_err());
            return;
        }
        let mut config = result.unwrap();
        assert_eq!(config.hunspell.is_some(), cfg!(feature = "hunspell"));
        assert_eq!(
            config.spellbook.is_some(),
            cfg!(all(not(feature = "hunspell"), feature = "spellbook"))
        );
        assert_eq!(
            config.zet.is_some(),
            cfg!(all(
                not(feature = "hunspell"),
                not(feature = "spellbook"),
                feature = "zet"
            ))
        );
        assert!(config.nlprules.is_none());
        for spelling in [&mut config.hunspell, &mut config.spellbook, &mut config.zet]
            .into_iter()
            .flatten()
        {
            spelling.skip_os_lookups = true;
            spelling.use_builtin = true;
        }
        let checkers = Checkers::new(config).unwrap();
        let documents = crate::Documentation::load_from_str(
            crate::ContentOrigin::TestEntityRust,
            "/// Zzyzxqvocab\nstruct Example;",
            true,
            false,
        );
        let (origin, chunks) = documents.iter().next().unwrap();
        let suggestions = checkers.check(origin, chunks).unwrap();
        assert!(suggestions.iter().any(|suggestion| {
            crate::sub_chars(suggestion.chunk.as_str(), suggestion.range.clone()) == "Zzyzxqvocab"
        }));
    }

    #[test]
    fn check_and_fix_require_a_compiled_checker() {
        for command in [
            "cargo spellcheck --checkers=reflow",
            "cargo spellcheck check --checkers=reflow",
            "cargo spellcheck fix --checkers=reflow",
            "cargo spellcheck --fix --checkers=reflow",
        ] {
            let error = load_test_config(command).unwrap_err();
            assert!(error.to_string().contains("No compiled checker is enabled"));
        }
        for (compiled, command) in [
            (
                cfg!(feature = "hunspell"),
                "cargo spellcheck --checkers=hunspell",
            ),
            (
                cfg!(feature = "spellbook"),
                "cargo spellcheck --checkers=spellbook",
            ),
            (cfg!(feature = "zet"), "cargo spellcheck --checkers=zet"),
            (
                cfg!(feature = "nlprules"),
                "cargo spellcheck --checkers=nlprules",
            ),
        ] {
            assert_eq!(load_test_config(command).is_ok(), compiled, "{command}");
        }
    }

    #[test]
    fn explicit_checker_filters_select_only_requested_compiled_backends() {
        for (command, hunspell, spellbook, zet, nlprules) in [
            (
                "cargo spellcheck --checkers=hunspell",
                true,
                false,
                false,
                false,
            ),
            (
                "cargo spellcheck --checkers=spellbook",
                false,
                true,
                false,
                false,
            ),
            ("cargo spellcheck --checkers=zet", false, false, true, false),
            (
                "cargo spellcheck --checkers=nlprules",
                false,
                false,
                false,
                true,
            ),
            (
                "cargo spellcheck --checkers=hunspell,spellbook,zet,nlprules",
                true,
                true,
                true,
                true,
            ),
        ] {
            let expected = [
                hunspell && cfg!(feature = "hunspell"),
                spellbook && cfg!(feature = "spellbook"),
                zet && cfg!(feature = "zet"),
                nlprules && cfg!(feature = "nlprules"),
            ];
            let result = load_test_config(command);
            if expected.iter().any(|enabled| *enabled) {
                let config = result.unwrap();
                assert_eq!(
                    [
                        config.hunspell.is_some(),
                        config.spellbook.is_some(),
                        config.zet.is_some(),
                        config.nlprules.is_some()
                    ],
                    expected,
                    "{command}",
                );
            } else {
                assert!(result.is_err(), "{command}");
            }
        }
    }

    #[test]
    fn nonchecking_commands_do_not_require_a_checker() {
        for command in [
            "cargo spellcheck reflow --checkers=reflow",
            "cargo spellcheck config --stdout",
            "cargo spellcheck list-files",
            "cargo spellcheck completions --shell=zsh",
        ] {
            assert!(load_test_config(command).is_ok(), "{command}");
        }
    }

    #[test]
    fn config_filter_accepts_all_compiled_checkers() {
        let mut config = Config::full();
        let reflow_width = config.reflow.as_ref().unwrap().max_line_length;
        let checkers = [
            CheckerType::Hunspell,
            CheckerType::NlpRules,
            CheckerType::ZSpell,
            CheckerType::Spellbook,
            CheckerType::Reflow,
            CheckerType::Hunspell,
        ];
        Args::checker_selection_override(Some(&checkers), &mut config).unwrap();
        assert_eq!(config.reflow.unwrap().max_line_length, reflow_width);
        assert_eq!(config.hunspell.is_some(), cfg!(feature = "hunspell"));
        assert_eq!(config.spellbook.is_some(), cfg!(feature = "spellbook"));
        assert_eq!(config.zet.is_some(), cfg!(feature = "zet"));
        assert_eq!(config.nlprules.is_some(), cfg!(feature = "nlprules"));
    }

    #[test]
    fn config_filter_requires_a_retained_compiled_checker() {
        for (checker, compiled) in [
            (CheckerType::Hunspell, cfg!(feature = "hunspell")),
            (CheckerType::NlpRules, cfg!(feature = "nlprules")),
            (CheckerType::ZSpell, cfg!(feature = "zet")),
            (CheckerType::Spellbook, cfg!(feature = "spellbook")),
            (CheckerType::Reflow, false),
        ] {
            let mut config = Config::full();
            let reflow_width = config.reflow.as_ref().unwrap().max_line_length;
            let result = Args::checker_selection_override(Some(&[checker]), &mut config);
            assert_eq!(result.is_ok(), compiled, "{checker:?}");
            assert_eq!(
                config.reflow.as_ref().unwrap().max_line_length,
                reflow_width
            );
            assert_eq!(
                config.has_spelling_backends(),
                compiled && checker != CheckerType::NlpRules
            );
        }
        let mut config = Config::full();
        assert!(Args::checker_selection_override(Some(&[]), &mut config).is_err());
        let mut config = Config::full();
        config.hunspell = None;
        config.zet = None;
        config.spellbook = None;
        config.nlprules = None;
        assert!(Args::checker_selection_override(None, &mut config).is_ok());
        assert!(Args::checker_selection_override(
            Some(&[CheckerType::Hunspell, CheckerType::NlpRules]),
            &mut config,
        )
        .is_err());
    }

    #[test]
    fn shell_check_env() {
        assert_matches!(load_shell_name("/usr/bin/zsh"), Ok(Shell::Zsh));
        assert_matches!(load_shell_name("zsh"), Ok(Shell::Zsh));
        assert_matches!(load_shell_name("fish"), Ok(Shell::Fish));

        static C1: &str = "cargo spellcheck completions --shell zsh";
        assert_matches!(Args::parse(commandline_to_iter(C1)), Ok(Args {
            command: Some(Sub::Completions { shell }),
            ..
        }) => {
            assert_eq!(shell.to_string(), "zsh")
        });

        // `SHELL` is builtin, and hence cannot be unset
        // so a negative test is impossible, an override works though
        // `std::env::remove_var("SHELL");`
        static C2: &str = "cargo spellcheck completions";

        std::env::set_var("SHELL", "/bin/fish");
        assert_matches!(Args::parse(commandline_to_iter(C2)), Ok(Args {
            command: Some(Sub::Completions { shell }),
            ..
        }) => {
            assert_eq!(shell.to_string(), "fish")
        });
    }
}
