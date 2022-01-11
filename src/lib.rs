#![deny(dead_code)]
#![deny(missing_docs)]
// #![deny(unused_crate_dependencies)]
#![deny(clippy::dbg_macro)]
#![warn(clippy::pedantic)]
#![allow(clippy::non_ascii_literal)]

//! cargo-spellcheck
//!
//! A syntax tree based doc comment and common mark spell checker.

pub mod action;
mod checker;
mod config;
mod documentation;
pub mod errors;
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

use self::errors::{bail, Result};

use log::{debug, info, trace, warn};
use serde::Deserialize;

use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};

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
    // Failure is already default for `Err(_)`
}

impl ExitCode {
    /// Convert ExitCode to primitive.
    pub fn as_u8(&self) -> u8 {
        match *self {
            Self::Success => 0u8,
            Self::Signal => 130u8,
            Self::Custom(code) => code,
        }
    }
}

/// Global atomic to block signal processing while a file write is currently in progress.
static WRITE_IN_PROGRESS: AtomicU16 = AtomicU16::new(0);
/// Delay if the signal handler is currently running.
static SIGNAL_HANDLER_AT_WORK: AtomicBool = AtomicBool::new(false);

/// Handle incoming signals.
///
/// Only relevant for *-nix platforms.
#[cfg(not(target_os = "windows"))]
pub fn signal_handler() {
    let mut signals =
        iterator::Signals::new(&[SIGTERM, SIGINT, SIGQUIT]).expect("Failed to create Signals");

    std::thread::spawn(move || {
        for s in signals.forever() {
            match s {
                SIGTERM | SIGINT | SIGQUIT => {
                    SIGNAL_HANDLER_AT_WORK.store(true, Ordering::SeqCst);
                    // Wait for potential writing to disk to be finished.
                    while WRITE_IN_PROGRESS.load(Ordering::Acquire) > 0 {
                        std::hint::spin_loop();
                        std::thread::yield_now();
                    }
                    if let Err(e) = action::interactive::ScopedRaw::restore_terminal() {
                        warn!("Failed to restore terminal: {}", e);
                    }
                    signal_hook::low_level::exit(130);
                }
                sig => warn!("Received unhandled signal {}, ignoring", sig),
            }
        }
    });
}

/// Blocks (unix) signals.
pub struct TinHat;

impl TinHat {
    /// Put the tin hat on, and only allow signals being processed once it's dropped.
    pub fn on() -> Self {
        while SIGNAL_HANDLER_AT_WORK.load(Ordering::Acquire) {
            std::hint::spin_loop();
            std::thread::yield_now();
        }
        let _ = WRITE_IN_PROGRESS.fetch_add(1, Ordering::Release);
        Self
    }
}

impl Drop for TinHat {
    fn drop(&mut self) {
        let _ = WRITE_IN_PROGRESS.fetch_sub(1, Ordering::Release);
    }
}

/// The inner main.
pub fn run() -> Result<ExitCode> {
    let args = Args::parse(std::env::args()).unwrap_or_else(|e| e.exit());

    let _ = ::rayon::ThreadPoolBuilder::new()
        .num_threads(args.job_count())
        .build_global();

    env_logger::Builder::from_env(env_logger::Env::new().filter_or("CARGO_SPELLCHECK", "warn"))
        .filter_level(args.verbosity())
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
    signal_handler();

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
                        bail!(
                            "Attempting to overwrite {} requires `--force`.",
                            path.display()
                        );
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

            let documents =
                traverse::extract(paths, recursive, skip_readme, dev_comments, &config)?;

            let rt = tokio::runtime::Runtime::new()?;
            let finish = rt.block_on(async move { action.run(documents, config).await })?;

            match finish {
                Finish::MistakeCount(0) => Ok(ExitCode::Success),
                Finish::MistakeCount(_n) => Ok(ExitCode::Custom(exit_code_override)),
                Finish::Abort => Ok(ExitCode::Signal),
                Finish::Success => Ok(ExitCode::Success),
            }
        }
    }
}
