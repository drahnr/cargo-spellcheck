use std::{marker::PhantomData, path::PathBuf};

use crate::errors::*;
use docopt::Docopt;

use crate::traverse;
use itertools::Itertools;
use serde::de::{self, DeserializeOwned, Deserializer};
use serde::Deserialize;
use std::fmt;
use std::result;
use std::str::FromStr;

use crate::Action;

use super::Config;

use log::{debug, info, warn};

/// Docopt usage string.
const USAGE: &str = r#"
Spellcheck all your doc comments

Usage:
    cargo-spellcheck [(-v...|-q)] [--jobs=<jobs>] fix [--cfg=<cfg>] [--code=<code>] [--dev-comments] [--skip-readme] [--checkers=<checkers>] [[--recursive] <paths>... ]
    cargo-spellcheck [(-v...|-q)] [--jobs=<jobs>] reflow [--cfg=<cfg>] [--code=<code>] [--dev-comments] [--skip-readme] [[--recursive] <paths>... ]
    cargo-spellcheck [(-v...|-q)] [--jobs=<jobs>] config (--user|--stdout|--cfg=<cfg>) [--checkers=<checkers>] [--force]
    cargo-spellcheck [(-v...|-q)] [--jobs=<jobs>] list-files [--skip-readme] [[--recursive] <paths>... ]
    cargo-spellcheck [(-v...|-q)] [--jobs=<jobs>] [check] [--fix] [--cfg=<cfg>] [--code=<code>] [--dev-comments] [--skip-readme] [--checkers=<checkers>] [[--recursive] <paths>... ]
    cargo-spellcheck --version
    cargo-spellcheck --help

Options:
  -h --help                 Show this screen.
  --version                 Print the version and exit.

  --fix                     Interactively apply spelling and grammer fixes, synonym to `fix` sub-command.
  -r --recursive            If a path is provided, if recursion into subdirectories is desired.
  --checkers=<checkers>     Calculate the intersection between
                            configured by config file and the ones provided on commandline.
  -f --force                Overwrite any existing configuration file. [default=false]
  -c --cfg=<cfg>            Use a non default configuration file.
                            Passing a directory will attempt to open `cargo_spellcheck.toml` in that directory.
  --user                    Write the configuration file to the default user configuration directory.
  --stdout                  Print the configuration file to stdout and exit.
  -v --verbose              Verbosity level.
  -q --quiet                Silences all printed messages. Overrules `-v`.
  -j --jobs=<jobs>          The number of threads to use for parallel checking.
  -m --code=<code>          Overwrite the exit value for a successful run with content mistakes found. [default=0]
  --skip-readme             Do not attempt to process README.md files listed in Cargo.toml manifests.
"#;

/// Checker types to be derived from the stringly typed arguments.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Deserialize)]
pub enum CheckerType {
    Hunspell,
    NlpRules,
    Reflow,
}

impl FromStr for CheckerType {
    type Err = UnknownCheckerTypeVariant;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_lowercase();
        Ok(match s.as_str() {
            "nlprules" => Self::NlpRules,
            "hunspell" => Self::Hunspell,
            "reflow" => Self::Reflow,
            _other => return Err(UnknownCheckerTypeVariant(s)),
        })
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("Unknown checker type variant: {0}")]
pub struct UnknownCheckerTypeVariant(String);

fn deser_option_vec_from_str_list<'de, T, D>(
    deserializer: D,
) -> result::Result<Option<Vec<T>>, D::Error>
where
    T: DeserializeOwned + fmt::Debug + FromStr,
    <T as FromStr>::Err: fmt::Display + fmt::Debug,
    D: Deserializer<'de>,
{
    deserializer.deserialize_option(OptionalVecOf::<T>::new())
}

#[derive(Debug, Clone, Copy)]
struct OptionalVecOf<T>(PhantomData<T>);

impl<T> OptionalVecOf<T> {
    fn new() -> Self {
        Self(PhantomData)
    }
}
impl<'de, T> de::Visitor<'de> for OptionalVecOf<T>
where
    T: fmt::Debug + de::DeserializeOwned + FromStr,
    <T as FromStr>::Err: fmt::Display + fmt::Debug,
{
    type Value = Option<Vec<T>>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "Expected a , separated string vector")
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(None)
    }

    fn visit_some<D>(self, deserializer: D) -> result::Result<Option<Vec<T>>, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(s.split(',')
            .into_iter()
            .map(|segment| <T as FromStr>::from_str(segment))
            .collect::<Result<Vec<_>, _>>()
            .map_err(serde::de::Error::custom)?)
        .map(|v| Some(v))
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct Args {
    pub arg_paths: Vec<PathBuf>,
    pub flag_fix: bool,
    pub flag_recursive: bool,
    pub flag_verbose: usize,
    pub flag_quiet: bool,
    pub flag_version: bool,
    pub flag_help: bool,
    #[serde(deserialize_with = "deser_option_vec_from_str_list")]
    pub flag_checkers: Option<Vec<CheckerType>>,
    pub flag_cfg: Option<PathBuf>,
    pub flag_force: bool,
    pub flag_user: bool,
    // with fallback from config, so it has to be tri-state
    pub flag_skip_readme: Option<bool>,
    pub flag_dev_comments: Option<bool>,
    pub flag_jobs: Option<usize>,
    pub flag_code: u8,
    pub flag_stdout: bool,
    pub cmd_fix: bool,
    pub cmd_check: bool,
    pub cmd_reflow: bool,
    pub cmd_config: bool,
    pub cmd_list_files: bool,
}

impl Args {
    pub const USAGE: &'static str = USAGE;

    /// Extract the verbosity level
    pub fn verbosity(&self) -> log::LevelFilter {
        match self.flag_verbose {
            _ if self.flag_quiet => log::LevelFilter::Off,
            n if n > 4 => log::LevelFilter::Trace,
            4 => log::LevelFilter::Debug,
            3 => log::LevelFilter::Info,
            2 => log::LevelFilter::Warn,
            _ => log::LevelFilter::Error,
        }
    }

    /// Extract the required action.
    pub fn action(&self) -> Action {
        // extract operation mode
        let action = if self.flag_help {
            Action::Help
        } else if self.flag_version {
            Action::Version
        } else if self.cmd_fix || self.flag_fix {
            Action::Fix
        } else if self.cmd_reflow {
            Action::Reflow
        } else if self.cmd_config {
            Action::Config
        } else if self.cmd_check {
            Action::Check
        } else if self.cmd_list_files {
            Action::ListFiles
        } else {
            // `cargo spellcheck` is short for checking
            Action::Check
        };
        log::trace!("Derived action {:?} from flags/args/cmds", action);
        action
    }

    /// Set the worker pool job/thread count.
    ///
    /// Affects the parallel processing for a particular checker.
    /// Checkers are always executed in sequence.
    pub fn job_count(&self) -> usize {
        match self.flag_jobs {
            _ if cfg!(debug_assertions) => {
                log::warn!("Debug mode always uses 1 thread!");
                1
            }
            Some(jobs) if jobs == 0 => {
                log::warn!(
                    "Cannot have less than one worker thread ({}). Retaining one worker thread.",
                    jobs
                );
                1
            }
            Some(jobs) if jobs > 128 => {
                log::warn!(
                    "Setting threads beyond 128 ({}) is insane. Capping at 128",
                    jobs
                );
                128
            }
            Some(jobs) => {
                log::info!("Explicitly set threads to {}", jobs);
                jobs
            }
            None => {
                // commonly we are not the only process
                // on the machine, so use the physical cores.
                let jobs = num_cpus::get_physical();
                log::debug!("Using the default physical thread count of {}", jobs);
                jobs
            }
        }
    }

    /// Adjust the raw arguments for call variants.
    ///
    /// The program could be called like `cargo-spellcheck`, `cargo spellcheck` or
    /// `cargo spellcheck check` and even ``cargo-spellcheck check`.
    pub fn parse(argv_iter: impl IntoIterator<Item = String>) -> Result<Self, docopt::Error> {
        Docopt::new(USAGE).and_then(|d| {
            // if ends with file name `cargo-spellcheck`
            let mut argv_iter = argv_iter.into_iter();
            if let Some(arg0) = argv_iter.next() {
                match PathBuf::from(&arg0)
                    .file_name()
                    .map(|x| x.to_str())
                    .flatten()
                {
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
                        let collected = next.into_iter().chain(argv_iter);
                        d.argv(collected)
                    }
                    _ => d,
                }
            } else {
                d
            }
            .deserialize()
        })
    }

    /// Overrides the enablement status of checkers in the configuration
    /// based on the checkers enabled by argument, if it is set.
    ///
    /// Errors of no checkers are left.
    pub fn checker_selection_override(
        filter_set: Option<&[CheckerType]>,
        config: &mut Config,
    ) -> Result<()> {
        // overwrite checkers
        if let Some(ref checkers) = filter_set {
            #[cfg(feature = "hunspell")]
            if !checkers.contains(&CheckerType::Hunspell) {
                if !config.hunspell.take().is_some() {
                    warn!("Hunspell was never configured.")
                }
            }
            #[cfg(feature = "nlprule")]
            if !checkers.contains(&CheckerType::NlpRules) {
                if !config.nlprules.take().is_some() {
                    warn!("Nlprules checker was never configured.")
                }
            }

            if !checkers.contains(&CheckerType::Reflow) {
                warn!("Reflow is a separate sub command.")
            }

            const EXPECTED_COUNT: usize =
                1_usize + cfg!(feature = "nlprule") as usize + cfg!(feature = "hunspell") as usize;

            if checkers.iter().unique().count() == EXPECTED_COUNT {
                bail!("Argument override for checkers disabled all checkers")
            }
        }
        Ok(())
    }

    /// Load configuration with fallbacks.
    ///
    /// Does IO checks if files exist.
    ///
    /// Provides a config and where it was retrieved from,
    /// if no config file exists, a default is provided
    /// and the config path becomes `None`.
    // TODO split the IO operations and lookup dirs.
    fn load_config(&self) -> Result<(Config, Option<PathBuf>)> {
        let (explicit_cfg, config_path) = match self.flag_cfg.as_ref() {
            Some(config_path) => {
                let config_path = if config_path.is_absolute() {
                    config_path.to_owned()
                } else {
                    crate::traverse::cwd()?.join(config_path)
                };
                (true, config_path)
            }
            None => {
                // TODO refactor needed

                // the current work dir as fallback
                let cwd = traverse::cwd()?;
                let mut config_path: PathBuf = cwd.as_path().join("Cargo.toml");

                // TODO Currently uses the first manifest dir as search dir for a spellcheck.toml
                // TODO with a fallback to the cwd as project dir.
                // TODO But it would be preferable to use the config specific to each dir if available.
                for path in self.arg_paths.iter() {
                    let path = if let Some(path) = if path.is_absolute() {
                        path.to_owned()
                    } else {
                        traverse::cwd()?.join(path)
                    }
                    .canonicalize()
                    .ok()
                    {
                        path
                    } else {
                        warn!(
                            "Provided path could not be canonicalized {}",
                            path.display()
                        );
                        // does not exist or access issues
                        continue;
                    };

                    if path.is_dir() {
                        let path = path.join("Cargo.toml");
                        if path.is_file() {
                            debug!("Using {} manifest as anchor file", path.display());
                            config_path = path;
                            break;
                        }
                    } else if let Some(file_name) = path.file_name() {
                        if file_name == "Cargo.toml" && path.is_file() {
                            debug!("Using {} manifest as anchor file", path.display());
                            config_path = path.to_owned();
                            break;
                        }
                    }
                    // otherwise it's a file and we do not care about it
                }

                // remove the file name
                let config_path = config_path.with_file_name("");

                let config_path = Config::project_config(&config_path)
                    .or_else(|e| {
                        debug!("Manifest dir found {}: {}", config_path.display(), e);
                        // in case there is none, attempt the cwd first before falling back to the user config
                        // this is a common case for workspace setups where we want to sanitize a sub project
                        Config::project_config(cwd.as_path())
                    })
                    .or_else(|e| {
                        debug!("Fallback to user default lookup, failed to load project specific config {}: {}", config_path.display(), e);
                        Config::default_path()
                    })?;
                (false, config_path)
            }
        };
        info!(
            "Attempting to use configuration file {}",
            config_path.display()
        );
        let (mut config, config_path) = match Config::load_from(&config_path) {
            Ok(config) => (config, Some(config_path)),
            Err(e) => {
                if explicit_cfg {
                    return Err(e);
                } else {
                    debug!(
                        "Loading configuration from {} failed due to: {}",
                        config_path.display(),
                        e
                    );
                    warn!(
                        "Loading configuration from {} failed, falling back to default values",
                        config_path.display(),
                    );
                    (Config::default(), None)
                }
            }
        };
        // mask all disabled checkers, use the default config
        // for those which have one if not enabled already
        if let Some(filter_set) = &self.flag_checkers {
            if filter_set.contains(&CheckerType::Hunspell) {
                if config.hunspell.is_none() {
                    config.hunspell = Some(crate::config::HunspellConfig::default());
                }
            } else {
                config.hunspell = None;
            }
            if filter_set.contains(&CheckerType::NlpRules) {
                if config.nlprules.is_none() {
                    config.nlprules = Some(crate::config::NlpRulesConfig::default());
                }
            } else {
                config.nlprules = None;
            }
            // reflow is a different subcommand, not relevant
        }

        Ok((config, config_path))
    }

    /// Evaluate the configuration flags, overwrite
    /// config values as needed and provide a new,
    /// unified config struct.
    pub fn unified(self) -> Result<(UnifiedArgs, Config)> {
        let (config, config_path) = self.load_config()?;

        let unified = match self.action() {
            Action::Config => {
                let dest_config = match self.flag_cfg {
                    None if self.flag_stdout => ConfigWriteDestination::Stdout,
                    Some(path) => ConfigWriteDestination::File {
                        overwrite: self.flag_force,
                        path: path.to_owned(),
                    },
                    None if self.flag_user => ConfigWriteDestination::File {
                        overwrite: self.flag_force,
                        path: Config::default_path()?,
                    },
                    _ => bail!("Neither --user or --stdout are given, invalid flags passed."),
                };
                UnifiedArgs::Config {
                    dest_config,
                    checker_filter_set: self.flag_checkers,
                }
            }
            action => UnifiedArgs::Operate {
                action,
                config_path,
                dev_comments: self.flag_dev_comments.unwrap_or(config.dev_comments),
                skip_readme: self.flag_skip_readme.unwrap_or(config.skip_readme),
                recursive: self.flag_recursive,
                paths: self.arg_paths,
                exit_code_override: self.flag_code,
            },
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
/// Only contains options which are either
/// only present in the arguments, or
/// are present in the arguments and have a fallback
/// in the configuration.
#[derive(Debug, Clone)]
pub enum UnifiedArgs {
    Config {
        dest_config: ConfigWriteDestination,
        checker_filter_set: Option<Vec<CheckerType>>,
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
            Self::Config { .. } => Action::Config,
            Self::Operate { action, .. } => *action,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;

    fn commandline_to_iter(s: &'static str) -> impl Iterator<Item = String> {
        s.split(' ').map(|s| s.to_owned()).into_iter()
    }

    lazy_static::lazy_static!(
        static ref SAMPLES: std::collections::HashMap<&'static str, Action> = maplit::hashmap!{
            "cargo spellcheck" => Action::Check,
            "cargo-spellcheck --version" => Action::Version,
            "cargo spellcheck --version" => Action::Version,
            "cargo spellcheck reflow" => Action::Reflow,
            "cargo spellcheck -vvvv" => Action::Check,
            "cargo spellcheck --fix" => Action::Fix,
            "cargo spellcheck fix" => Action::Fix,
            "cargo-spellcheck" => Action::Check,
            "cargo-spellcheck -vvvv" => Action::Check,
            "cargo-spellcheck --fix" => Action::Fix,
            "cargo-spellcheck fix" => Action::Fix,
            "cargo-spellcheck fix -r file.rs" => Action::Fix,
            "cargo-spellcheck -q fix Cargo.toml" => Action::Fix,
            "cargo spellcheck -v fix Cargo.toml" => Action::Fix,
            "cargo spellcheck -m 11 check" => Action::Check,
            "cargo-spellcheck reflow" => Action::Reflow,
        };
    );

    #[test]
    fn docopt() {
        for command in SAMPLES.keys() {
            assert!(Args::parse(commandline_to_iter(command))
                .map_err(|e| {
                    println!("Processing > {:?}", command);
                    e
                })
                .is_ok());
        }
    }

    #[test]
    fn deserialize_multiple_checkers() {
        let args = Args::parse(commandline_to_iter(
            "cargo spellcheck check --checkers=nlprules,hunspell",
        ))
        .expect("Parsing works. qed");
        assert_eq!(
            args.flag_checkers,
            Some(vec![CheckerType::NlpRules, CheckerType::Hunspell])
        );
    }

    #[test]
    fn unify_ops_check() {
        let args = Args::parse(
            &mut [
                "cargo",
                "spellcheck",
                "-vvvvv",
                "check",
                "--code=77",
                "--dev-comments",
                "--skip-readme",
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

    #[test]
    fn unify_config() {
        let args = Args::parse(
            &mut [
                "cargo-spellcheck",
                "config",
                "--cfg=.config/spellcheck.toml",
                "--checkers=NlpRules",
                "--force",
            ]
            .iter()
            .map(ToOwned::to_owned)
            .map(ToOwned::to_owned),
        )
        .unwrap();
        let (unified, config) = args.unified().unwrap();
        assert_matches!(unified,
            UnifiedArgs::Config {
                dest_config: ConfigWriteDestination::File { overwrite, path },
                checker_filter_set,
            } => {
                assert_eq!(path, PathBuf::from(".config/spellcheck.toml"));
                assert_eq!(checker_filter_set, Some(vec![CheckerType::NlpRules]));
                assert_eq!(overwrite, true);
            }
        );

        assert_matches!(config.hunspell, None => {});
        assert_matches!(config.nlprules, Some(cfg) => {
            assert!(cfg.override_rules.is_none());
            assert!(cfg.override_tokenizer.is_none());
        });
    }
}
