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
pub use self::config::args::*;
pub use self::config::{Config, HunspellConfig, LanguageToolConfig};
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

/// A simple exit code representation.
///
/// `Custom` can be specified by the user, others map to their UNIX equivalents
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
    let args = Args::parse(std::env::args()).unwrap_or_else(|e| e.exit());

    let _ = ::rayon::ThreadPoolBuilder::new()
        .num_threads(args.job_count())
        .build_global();

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
        .filter_module("nlprule", log::LevelFilter::Error)
        .filter_module("mio", log::LevelFilter::Error)
        .init();

    // handle the simple variants right away
    match args.action() {
        Action::Version => {
            println!("cargo-spellcheck {}", env!("CARGO_PKG_VERSION"));
            return Ok(ExitCode::Success);
        }
        Action::Help => {
            println!("{}", Args::USAGE);
            return Ok(ExitCode::Success);
        }
        _ => {}
    }

    #[cfg(not(target_os = "windows"))]
    let _signalthread = std::thread::spawn(move || signal_handler());

    let (unified, config) = args.unified()?;

    match unified {
        // must unify first, for the proper paths
        UnifiedArgs::Config {
            dest_config,
            checker_filter_set,
        } => {
            trace!("Configuration chore");
            let mut config = Config::full();
            Args::checker_selection_override(
                checker_filter_set.as_ref().map(|x| x.as_slice()),
                &mut config,
            )?;

            match dest_config {
                ConfigWriteDestination::Stdout => {
                    println!("{}", config.to_toml()?);
                    return Ok(ExitCode::Success);
                }
                ConfigWriteDestination::File { overwrite, path } => {
                    if path.exists() && !overwrite {
                        return Err(anyhow::anyhow!(
                            "Attempting to overwrite {} requires `--force`.",
                            path.display()
                        ));
                    }

                    info!("Writing configuration file to {}", path.display());
                    config.write_values_to_path(path)?;
                }
            }
            return Ok(ExitCode::Success);
        }
        UnifiedArgs::Operate {
            action,
            paths,
            recursive,
            skip_readme,
            config_path,
            dev_comments,
            exit_code_override,
        } => {
            debug!(
                "Executing: {:?} with {:?} from {:?}",
                action, &config, config_path
            );

            let combined = traverse::extract(paths, recursive, skip_readme, dev_comments, &config)?;

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
                Finish::MistakeCount(_n) => Ok(ExitCode::Custom(exit_code_override)),
                Finish::Abort => Ok(ExitCode::Signal),
            }
        }
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
