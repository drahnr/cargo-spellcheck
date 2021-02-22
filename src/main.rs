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
pub use self::config::args::*;
pub use self::documentation::*;
pub use self::span::*;
pub use self::suggestion::*;
pub use self::util::*;

use log::{debug, info, trace, warn};
use serde::Deserialize;

#[cfg(not(target_os = "windows"))]
use signal_hook::{
    consts::signal::{SIGINT, SIGQUIT, SIGTERM},
    iterator,
};

#[cfg(target_os = "windows")]
use signal_hook as _;

use checker::Checker;
use std::path::PathBuf;


/// A simple exit code representation.
///
/// `Custom` can be specified by the user, others map to thei unix equivalents
/// where available.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ExitCode {
    /// Regular termination and does not imply anything in regards to spelling
    /// mistakes found or not.
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


/// Handle incoming signals.
///
/// Only relevant for *-nix platforms.
#[cfg(not(target_os = "windows"))]
fn signal_handler() {
    let mut signals =
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


/// The inner main.
fn run() -> anyhow::Result<ExitCode> {
    #[cfg(debug_assertions)]
    let _ = ::rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build_global();

    let args = Args::parse(std::env::args()).unwrap_or_else(|e| e.exit());

    let verbosity = match args.flag_verbose {
        _ if args.flag_quiet => log::LevelFilter::Off,
        n if n > 4 => log::LevelFilter::Trace,
        4 => log::LevelFilter::Debug,
        3 => log::LevelFilter::Info,
        2 => log::LevelFilter::Warn,
        _ => log::LevelFilter::Error,
    };

    env_logger::Builder::from_env(env_logger::Env::new().filter_or("CARGO_SPELLCHECK", "warn"))
        .filter_level(verbosity)
        .init();

    let checkers = |config: &mut Config| {
        // overwrite checkers
        if let Some(ref checkers) = args.flag_checkers {
            if !checkers.contains(&CheckerType::Hunspell) {
                if !config.hunspell.take().is_some() {
                    warn!("Hunspell was never configured.")
                }
            }
            if !checkers.contains(&CheckerType::LanguageTool) {
                if !config.languagetool.take().is_some() {
                    warn!("Languagetool was never configured.")
                }
            }
            if !checkers.contains(&CheckerType::Reflow) {
                warn!("Reflow is a separate sub command.")
            }
        }
    };

    let action = match args.action() {
        Action::Version => {
            println!("cargo-spellcheck {}", env!("CARGO_PKG_VERSION"));
            return Ok(ExitCode::Success);
        }
        Action::Help => {
            println!("{}", Args::USAGE);
            return Ok(ExitCode::Success);
        }
        Action::Config => {
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
        }
        action => action,
    };

    #[cfg(not(target_os = "windows"))]
    let _signalthread = std::thread::spawn(move || signal_handler());

    let (explicit_cfg, config_path) = match args.flag_cfg.as_ref() {
        Some(config_path) => {
            let config_path = if config_path.is_absolute() {
                config_path.to_owned()
            } else {
                traverse::cwd()?.join(config_path)
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

            // remove the file name
            let config_path = config_path.with_file_name("");

            let resolved_config_path = config::Config::project_config(&config_path)
                .or_else(|e| {
                    debug!("Manifest dir found {}: {}", config_path.display(), e);
                    // in case there is none, attempt the cwd first before falling back to the user config
                    // this is a common case for workspace setups where we want to sanitize a sub project
                    config::Config::project_config(cwd.as_path())
                })
                .or_else(|e| {
                    debug!("Fallback to user default lookup, failed to load project specific config {}: {}", config_path.display(), e);
                    Config::default_path()
                })?;
            (false, resolved_config_path)
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

    debug!("Executing: {:?} with {:?}", action, &config);

    let dev_comments = args.flag_dev_comments.unwrap_or(config.dev_comments);
    let skip_readme = args.flag_skip_readme.unwrap_or(config.skip_readme);

    let combined = traverse::extract(
        args.arg_paths,
        args.flag_recursive,
        skip_readme,
        dev_comments,
        &config,
    )?;

    // TODO move this into action `fn run()`
    let suggestion_set = match action {
        Action::Reflow => {
            reflow::Reflow::check(&combined, &config.reflow.clone().unwrap_or_default())?
        }
        Action::Check | Action::Fix => checker::check(&combined, &config)?,
        _ => unreachable!("Should never be reached, handled earlier"),
    };

    let finish = action.run(suggestion_set, &config)?;

    match finish {
        Finish::MistakeCount(0) => Ok(ExitCode::Success),
        Finish::MistakeCount(_n) => Ok(ExitCode::Custom(args.flag_code)),
        Finish::Abort => Ok(ExitCode::Signal),
    }
}

#[allow(missing_docs)]
fn main() -> anyhow::Result<()> {
    let val = run()?.as_u8();
    if val != 0 {
        std::process::exit(val as i32)
    }
    Ok(())
}
