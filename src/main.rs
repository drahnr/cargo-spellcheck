#![deny(dead_code)]
#![deny(missing_docs)]
#![deny(unused_crate_dependencies)]
#![warn(clippy::pedantic)]

//! cargo-spellcheck
//!
//! A syntax tree based doc comment and common mark spell checker.

mod action;
mod checker;
mod config;
mod documentation;
mod reflow;
mod span;
mod suggestion;
mod traverse;
mod util;

pub use self::action::*;
pub use self::config::{Config, HunspellConfig, LanguageToolConfig};
pub use self::documentation::*;
pub use self::span::*;
pub use self::suggestion::*;
pub use self::util::*;

use docopt::Docopt;

use log::{debug, info, trace, warn};
use serde::Deserialize;

#[cfg(not(target_os = "windows"))]
use signal_hook::{iterator, SIGINT, SIGQUIT, SIGTERM};

#[cfg(target_os = "windows")]
use signal_hook as _;

use std::path::PathBuf;

/// Docopt usage string.
const USAGE: &str = r#"
Spellcheck all your doc comments

Usage:
    cargo-spellcheck [(-v...|-q)] check [--cfg=<cfg>] [--code=<code>] [--skip-readme] [--checkers=<checkers>] [[--recursive] <paths>... ]
    cargo-spellcheck [(-v...|-q)] fix [--cfg=<cfg>] [--code=<code>] [--skip-readme] [--checkers=<checkers>] [[--recursive] <paths>... ]
    cargo-spellcheck [(-v...|-q)] config (--user|--stdout|--cfg=<cfg>) [--force]
    cargo-spellcheck [(-v...|-q)] [--cfg=<cfg>] [--fix] [--code=<code>] [--skip-readme] [--checkers=<checkers>] [[--recursive] <paths>... ]
    cargo-spellcheck --help
    cargo-spellcheck --version

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

/// A simple exit code representation.
///
/// `Custom` can be specified by the user,
/// others map to thei unix equivalents where
/// available.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ExitCode {
    /// Regular termination and does not imply anything
    /// in regards to spelling mistakes found or not.
    Success,
    /// Terminate requested by a *nix signal.
    Signal,
    /// A custom exit code, as specified with `--code=<code>`.
    Custom(u8),
    // Failure is already default for `Err(anyhow::Error)`
}

impl ExitCode {
    fn as_u8(&self) -> u8 {
        match *self {
            Self::Success => 0u8,
            Self::Signal => 130u8,
            Self::Custom(code) => code,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct Args {
    arg_paths: Vec<PathBuf>,
    flag_fix: bool,
    flag_recursive: bool,
    flag_verbose: usize,
    flag_quiet: bool,
    flag_version: bool,
    flag_help: bool,
    flag_checkers: Option<String>,
    flag_cfg: Option<PathBuf>,
    flag_force: bool,
    flag_user: bool,
    flag_skip_readme: bool,
    flag_code: u8,
    flag_stdout: bool,
    cmd_fix: bool,
    cmd_check: bool,
    cmd_config: bool,
}

/// Handle incoming signals.
///
/// Only relevant for *-nix platforms.
#[cfg(not(target_os = "windows"))]
fn signal_handler() {
    let signals =
        iterator::Signals::new(vec![SIGTERM, SIGINT, SIGQUIT]).expect("Failed to create Signals");
    for s in signals.forever() {
        match s {
            SIGTERM | SIGINT | SIGQUIT => {
                if let Err(e) = action::interactive::ScopedRaw::restore_terminal() {
                    warn!("Failed to restore terminal: {}", e);
                }
                std::process::exit(130);
            }
            sig => warn!("Received unhandled signal {}, ignoring", sig),
        }
    }
}

/// Agjust the raw arguments for call variants.
///
/// The program could be called like `cargo-spellcheck`, `cargo spellcheck` or
/// `cargo spellcheck check` and even ``cargo-spellcheck check`.
fn parse_args(mut argv_iter: impl Iterator<Item = String>) -> Result<Args, docopt::Error> {
    Docopt::new(USAGE).and_then(|d| {
        // if ends with file name `cargo-spellcheck`, split
        if let Some(arg0) = argv_iter.next() {
            match PathBuf::from(&arg0)
                .file_name()
                .map(|x| x.to_str())
                .flatten()
            {
                Some(file_name) => {
                    // allow all variants
                    // cargo spellcheck ...
                    // cargo-spellcheck ...
                    // cargo-spellcheck spellcheck ...
                    let mut next = vec!["cargo-spellcheck".to_owned()];

                    match argv_iter.next() {
                        Some(arg)
                            if file_name.starts_with("cargo-spellcheck") && arg == "spellcheck" => {
                        }
                        Some(arg) => next.push(arg.to_owned()),
                        _ => {}
                    };
                    let collected = next.into_iter().chain(argv_iter).collect::<Vec<_>>();
                    d.argv(collected.into_iter())
                }
                _ => d,
            }
        } else {
            d
        }
        .deserialize()
    })
}

/// The inner main.
fn run() -> anyhow::Result<ExitCode> {
    let args = parse_args(std::env::args()).unwrap_or_else(|e| e.exit());

    let verbosity = match args.flag_verbose {
        _ if args.flag_quiet => log::LevelFilter::Off,
        n if n > 4 => log::LevelFilter::Trace,
        4 => log::LevelFilter::Debug,
        3 => log::LevelFilter::Info,
        2 => log::LevelFilter::Warn,
        _ => log::LevelFilter::Error,
    };

    env_logger::from_env(env_logger::Env::new().filter_or("CARGO_SPELLCHECK", "warn"))
        .filter_level(verbosity)
        .init();

    if args.flag_version {
        println!("cargo-spellcheck {}", env!("CARGO_PKG_VERSION"));
        return Ok(ExitCode::Success);
    }

    if args.flag_help {
        println!("{}", USAGE);
        return Ok(ExitCode::Success);
    }

    #[cfg(not(target_os = "windows"))]
    std::thread::spawn(move || signal_handler());

    let checkers = |config: &mut Config| {
        // overwrite checkers
        if let Some(checkers) = args.flag_checkers.clone() {
            let checkers = checkers
                .split(',')
                .map(|checker| checker.to_lowercase())
                .collect::<Vec<_>>();
            if !checkers.contains(&"hunspell".to_owned()) {
                if !config.hunspell.take().is_some() {
                    warn!("Hunspell was never configured.")
                }
            }
            if !checkers.contains(&"languagetool".to_owned()) {
                if !config.languagetool.take().is_some() {
                    warn!("Languagetool was never configured.")
                }
            }
        }
    };

    // handle `config` sub command
    if args.cmd_config {
        trace!("Configuration chore");
        let mut config = Config::full();
        checkers(&mut config);

        let config_path = match args.flag_cfg.as_ref() {
            Some(path) => Some(path.to_owned()),
            None if args.flag_user => Some(Config::default_path()?),
            None => None,
        };

        if args.flag_stdout {
            println!("{}", config.to_toml()?);
            return Ok(ExitCode::Success);
        }

        if let Some(path) = config_path {
            if path.is_file() && !args.flag_force {
                return Err(anyhow::anyhow!(
                    "Attempting to overwrite {} requires `--force`.",
                    path.display()
                ));
            }
            info!("Writing configuration file to {}", path.display());
            config.write_values_to_path(path)?;
        }
        return Ok(ExitCode::Success);
    } else {
        trace!("Not configuration sub command");
    }

    let (explicit_cfg, config_path) = match args.flag_cfg.as_ref() {
        Some(path) => {
            let path = if path.is_absolute() {
                path.to_owned()
            } else {
                traverse::cwd()?.join(path)
            };
            (true, path)
        }
        None => {
            // @todo refactor needed

            // the current work dir as fallback
            let mut config_path: PathBuf = traverse::cwd()?.join("Cargo.toml");

            // overwrite with the first found manifest
            for path in args.arg_paths.iter() {
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

            let config_path = config_path.with_file_name(""); //.expect("Found file ends in Cargo.toml and is abs. qed");

            let path = config::Config::project_config(&config_path)
                .or_else(|e| {
                    debug!("Fallback to user default lookup, failed to load project specific config {}: {}", config_path.display(), e);
                    Config::default_path()
                })?;
            (false, path)
        }
    };
    info!("Using configuration file {}", config_path.display());
    let mut config = match Config::load_from(&config_path) {
        Ok(config) => config,
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
                Config::default()
            }
        }
    };

    checkers(&mut config);

    // extract operation mode
    let action = if args.cmd_fix || args.flag_fix {
        Action::Fix
    } else {
        // check
        Action::Check
    };

    trace!("Executing: {:?} with {:?}", action, &config);

    let combined = traverse::extract(
        args.arg_paths,
        args.flag_recursive,
        args.flag_skip_readme,
        &config,
    )?;

    let suggestion_set = checker::check(&combined, &config)?;

    let finish = action.run(suggestion_set, &config)?;

    match finish {
        Finish::MistakeCount(0) => Ok(ExitCode::Success),
        Finish::MistakeCount(_n) => Ok(ExitCode::Custom(args.flag_code)),
        Finish::Abort => Ok(ExitCode::Signal),
    }
}

#[allow(missing_docs)]
fn main() -> anyhow::Result<()> {
    std::process::exit(run()?.as_u8() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commandline_to_iter(s: &'static str) -> impl Iterator<Item = String> {
        s.split(' ').map(|s| s.to_owned()).into_iter()
    }

    #[test]
    fn docopt() {
        let commands = vec![
            "cargo spellcheck",
            "cargo spellcheck -vvvv",
            "cargo spellcheck --fix",
            "cargo spellcheck fix",
            "cargo-spellcheck",
            "cargo-spellcheck -vvvv",
            "cargo-spellcheck --fix",
            "cargo-spellcheck fix",
            "cargo-spellcheck fix -r file.rs",
            "cargo-spellcheck -q fix Cargo.toml",
            "cargo spellcheck -v fix Cargo.toml",
            "cargo spellcheck -m 11 check",
        ];
        for command in commands {
            assert!(parse_args(commandline_to_iter(command)).is_ok());
        }
    }
}
