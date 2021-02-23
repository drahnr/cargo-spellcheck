use std::path::PathBuf;

use anyhow::{bail, Result};
use docopt::Docopt;

use crate::traverse;
use itertools::Itertools;
use serde::Deserialize;

use crate::Action;

use super::Config;

use log::{debug, info, warn};

/// Docopt usage string.
const USAGE: &str = r#"
Spellcheck all your doc comments

Usage:
    cargo-spellcheck [(-v...|-q)] fix [--cfg=<cfg>] [--code=<code>] [--dev-comments] [--skip-readme] [--checkers=<checkers>] [[--recursive] <paths>... ]
    cargo-spellcheck [(-v...|-q)] reflow [--cfg=<cfg>] [--code=<code>] [--dev-comments] [--skip-readme] [[--recursive] <paths>... ]
    cargo-spellcheck [(-v...|-q)] config (--user|--stdout|--cfg=<cfg>) [--checkers=<checkers>] [--force]
    cargo-spellcheck [(-v...|-q)] [check] [--fix] [--cfg=<cfg>] [--code=<code>] [--dev-comments] [--skip-readme] [--checkers=<checkers>] [[--recursive] <paths>... ]
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
  -m --code=<code>          Overwrite the exit value for a successful run with content mistakes found. [default=0]
  --skip-readme             Do not attempt to process README.md files listed in Cargo.toml manifests.
"#;

/// Checker types to be derived from the stringly typed arguments.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Deserialize)]
pub enum CheckerType {
    #[serde(alias = "hunspell")]
    #[serde(alias = "hunSpell")]
    Hunspell,
    #[serde(alias = "nlprules")]
    #[serde(alias = "nlpRules")]
    NlpRules,
    #[serde(alias = "languageTool")]
    #[serde(alias = "Languagetool")]
    #[serde(alias = "languagetool")]
    LanguageTool,
    #[serde(alias = "ReFlow")]
    #[serde(alias = "reflow")]
    Reflow,
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
    pub flag_checkers: Option<Vec<CheckerType>>,
    pub flag_cfg: Option<PathBuf>,
    pub flag_force: bool,
    pub flag_user: bool,
    // with fallback from config, so it has to be tri-state
    pub flag_skip_readme: Option<bool>,
    pub flag_dev_comments: Option<bool>,
    pub flag_code: u8,
    pub flag_stdout: bool,
    pub cmd_fix: bool,
    pub cmd_check: bool,
    pub cmd_reflow: bool,
    pub cmd_config: bool,
}

impl Args {
    pub const USAGE: &'static str = USAGE;

    /// Extract the required action.
    pub fn action(&self) -> Action {
        // extract operation mode
        let action = if self.cmd_fix {
            Action::Fix
        } else if self.flag_fix {
            Action::Fix
        } else if self.cmd_reflow {
            Action::Reflow
        } else if self.cmd_config {
            Action::Config
        } else if self.flag_help {
            Action::Help
        } else if self.flag_version {
            Action::Version
        } else if self.cmd_check {
            Action::Check
        } else {
            // `cargo spellcheck` is short for checking
            Action::Check
        };
        log::trace!("Derived action {:?} from flags/args/cmds", action);
        action
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
    /// based on the checkers enabled by arg, if it is set.
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
            #[cfg(feature = "languagetool")]
            if !checkers.contains(&CheckerType::LanguageTool) {
                if !config.languagetool.take().is_some() {
                    warn!("Languagetool checker was never configured.")
                }
            }

            if !checkers.contains(&CheckerType::Reflow) {
                warn!("Reflow is a separate sub command.")
            }

            const EXPECTED_COUNT: usize = 1_usize
                + cfg!(feature = "nlprule") as usize
                + cfg!(feature = "hunspell") as usize
                + cfg!(feature = "languagetool") as usize;

            if checkers.iter().unique().count() == EXPECTED_COUNT {
                bail!("Argument override for checkers disabled all checkers")
            }
        }
        Ok(())
    }

    /// Load configuration with fallbacks.
    ///
    /// When explicitly
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
        let (config, config_path) = match Config::load_from(&config_path) {
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
        let (unified, _config) = args.unified().unwrap();
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
    }
}
