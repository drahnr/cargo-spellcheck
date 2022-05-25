#![deny(dead_code)]
#![deny(missing_docs)]
// #![deny(unused_crate_dependencies)]

// Prevent the stray dbg! macros
#![deny(clippy::dbg_macro)]
#![warn(clippy::pedantic)]
#![allow(clippy::non_ascii_literal)]
// be explicit about certain offsets and how they are constructed
#![allow(clippy::identity_op)]
// in small cli projects, this is ok for now
#![allow(clippy::wildcard_imports)]

//! cargo-spellcheck
//!
//! A syntax tree based doc comment and common mark spell checker.

pub use doc_chunks as documentation;

pub mod action;
mod checker;
mod config;
pub mod errors;
mod reflow;
mod suggestion;
mod tinhat;
mod traverse;

pub use self::action::*;
pub use self::config::args::*;
pub use self::config::{Config, HunspellConfig, LanguageToolConfig};
pub use self::documentation::span::*;
pub use self::documentation::util::*;
pub use self::documentation::*;
pub use self::suggestion::*;
pub use self::tinhat::*;

use self::errors::{bail, Result};

use std::io::Write;

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
    /// Convert `ExitCode` to primitive.
    pub fn as_u8(&self) -> u8 {
        match *self {
            Self::Success => 0u8,
            Self::Signal => 130u8,
            Self::Custom(code) => code,
        }
    }
}

/// The inner main.
pub fn run(args: Args) -> Result<ExitCode> {
    let _ = ::rayon::ThreadPoolBuilder::new()
        .num_threads(args.job_count())
        .build_global();

    env_logger::Builder::from_env(env_logger::Env::new().filter_or("CARGO_SPELLCHECK", "warn"))
        .filter_level(args.verbosity())
        .filter_module("nlprule", log::LevelFilter::Error)
        .filter_module("mio", log::LevelFilter::Error)
        .init();

    #[cfg(not(target_os = "windows"))]
    signal_handler(move || {
        if let Err(e) = action::interactive::ScopedRaw::restore_terminal() {
            log::warn!("Failed to restore terminal: {}", e);
        }
    });

    let (unified, config) = match &args.command {
        Some(Sub::Completions { shell }) => {
            let sink = &mut std::io::stdout();
            generate_completions(*shell, sink);
            let _ = sink.flush();
            return Ok(ExitCode::Success);
        }
        _ => args.unified()?,
    };

    match unified {
        // must unify first, for the proper paths
        UnifiedArgs::Config {
            dest_config,
            checker_filter_set,
        } => {
            log::trace!("Configuration chore");
            let mut config = Config::full();
            Args::checker_selection_override(
                checker_filter_set.as_ref().map(AsRef::as_ref),
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

                    log::info!("Writing configuration file to {}", path.display());
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
            log::debug!(
                "Executing: {:?} with {:?} from {:?}",
                action,
                &config,
                config_path
            );

            let documents =
                traverse::extract(paths, recursive, skip_readme, dev_comments, &config)?;

            let rt = tokio::runtime::Runtime::new()?;
            let finish = rt.block_on(async move { action.run(documents, config).await })?;

            match finish {
                Finish::Success | Finish::MistakeCount(0) => Ok(ExitCode::Success),
                Finish::MistakeCount(_n) => Ok(ExitCode::Custom(exit_code_override)),
                Finish::Abort => Ok(ExitCode::Signal),
            }
        }
    }
}

#[cfg(test)]
mod tests;
