mod config;
mod documentation;
mod literalset;
mod span;

mod checker;
mod extractor;
mod suggestion;

pub use self::config::{Config, HunspellConfig, LanguageToolConfig};
pub use self::documentation::*;
pub use self::literalset::*;
pub use self::span::*;
pub use self::suggestion::*;

use docopt::Docopt;
use enumflags2::BitFlags;
use log::{debug, info, trace, warn};
use serde::Deserialize;

use std::path::PathBuf;

const USAGE: &str = r#"
Spellcheck all your doc comments

Usage:
    cargo-spellcheck check [[--recursive] <paths>.. ]
    cargo spellcheck check [[--recursive] <paths>.. ]
    cargo-spellcheck fix [[--recursive] <paths>.. ]
    cargo spellcheck fix [[--recursive] <paths>.. ]
    cargo-spellcheck [(--fix|--interactive)] [[--recursive] <paths>.. ]
    cargo spellcheck [(--fix|--interactive)] [[--recursive] <paths>.. ]
    cargo-spellcheck config [--overwrite]
    cargo spellcheck config [--overwrite]

Options:
  -h --help           Show this screen.
  --fix               Synonym to running the `fix` subcommand.
  -i --interactive    Interactively apply spelling and grammer fixes.
  -r --recursive      If a path is provided, if recursion into subdirectories is desired.
  --overwrite         Overwrite any existing configuration file.

"#;

#[derive(Debug, Deserialize, Default)]
struct Args {
    arg_paths: Vec<PathBuf>,
    flag_fix: bool,
    flag_interactive: bool,
    flag_recursive: bool,
    flag_overwrite: bool,
    cmd_fix: bool,
    cmd_check: bool,
    cmd_config: bool,
}

/// Mode in which we operate
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Mode {
    /// Fix issues without interaction if there is sufficient information
    Fix,
    /// Only show errors
    Check,
    /// Interactively choose from candidates provided, simliar to `git add -p` .
    Interactive,
}

fn main() -> anyhow::Result<()> {
    env_logger::init();

    if log::log_enabled!(log::Level::Trace) {
        let args: Vec<String> = std::env::args().collect();
        trace!("Args: {:?}", args);
    }

    let args: Args = Docopt::new(USAGE)
        .and_then(|d| d.deserialize())
        .unwrap_or_else(|e| e.exit());

    // handle `config` sub command
    if args.cmd_config {
        let config = Config::load().or_else(|e| {
            if args.flag_overwrite {
                Config::write_default_values()
            } else {
                Err(e)
            }
        })?;
        println!("{}", config.to_toml()?);
        return Ok(());
    }

    // extract operation mode
    let mode = if args.cmd_fix || args.flag_fix {
        Mode::Fix
    } else if args.flag_interactive {
        Mode::Interactive
    } else {
        // check
        Mode::Check
    };

    // do not write the config without an explicit request
    let config = Config::load().unwrap_or_else(|_e| {
        warn!("Using default configuration!");
        Config::default()
    });

    trace!("Executing: {:?}", mode);

    extractor::run(mode, args.arg_paths, args.flag_recursive, &config)
}
