mod config;
mod documentation;
mod literalset;
mod span;

mod checker;
mod markdown;
mod suggestion;
mod traverse;

pub use self::config::{Config, HunspellConfig, LanguageToolConfig};
pub use self::documentation::*;
pub use self::literalset::*;
pub use self::markdown::*;
pub use self::span::*;
pub use self::suggestion::*;

use docopt::Docopt;

use log::{trace, warn};
use serde::Deserialize;

use std::path::PathBuf;

const USAGE: &str = r#"
Spellcheck all your doc comments

Usage:
    cargo spellcheck [(-v...|-q)] check [[--recursive] <paths>.. ]
    cargo spellcheck [(-v...|-q)] fix [[--recursive] <paths>.. ]
    cargo spellcheck [(-v...|-q)] [(--fix|--interactive)] [[--recursive] <paths>.. ]
    cargo spellcheck [(-v...|-q)] config [--overwrite]
    cargo spellcheck --version

Options:
  -h --help           Show this screen.
  --version           Print the version and exit.

  --fix               Synonym to running the `fix` subcommand.
  -i --interactive    Interactively apply spelling and grammer fixes.
  -r --recursive      If a path is provided, if recursion into subdirectories is desired.
  --overwrite         Overwrite any existing configuration file.
  -v --verbose        Verbosity level.
  -q --quiet          Silences all printed messages.

"#;

#[derive(Debug, Deserialize, Default)]
struct Args {
    arg_paths: Vec<PathBuf>,
    flag_fix: bool,
    flag_interactive: bool,
    flag_recursive: bool,
    flag_overwrite: bool,
    flag_verbose: Vec<bool>,
    flag_quiet: bool,
    flag_version: bool,
    cmd_fix: bool,
    cmd_check: bool,
    cmd_config: bool,
    // allow both cargo_spellcheck and cargo spellcheck
    cmd_spellcheck: bool,
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
    let args: Args = Docopt::new(USAGE)
        .and_then(|d| {
            let mut argv_it = std::env::args();
            // if ends with file name `cargo-spellcheck`, split
            if let Some(arg0) = argv_it.next() {
                if let Some(file_name) = PathBuf::from(&arg0)
                    .file_name()
                    .map(|x| x.to_str())
                    .flatten()
                {
                    if file_name.starts_with("cargo-spellcheck") {
                        d.argv(
                            file_name
                                .split('-')
                                .skip(1)
                                .map(|x| x.to_owned())
                                .chain(argv_it),
                        )
                    } else {
                        d
                    }
                } else {
                    d
                }
            } else {
                d
            }
            .deserialize()
        })
        .unwrap_or_else(|e| e.exit());

    let mut builder = env_logger::from_env("CARGO_SPELLCHECK");
    let verbosity = match args.flag_verbose.len() {
        _ if args.flag_quiet => log::LevelFilter::Off,
        n if n > 4 => log::LevelFilter::Trace,
        4 => log::LevelFilter::Debug,
        3 => log::LevelFilter::Info,
        2 => log::LevelFilter::Warn,
        _ => log::LevelFilter::Error,
    };
    builder.filter_level(verbosity).init();

    if args.flag_version {
        println!("cargo-spellcheck {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

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

    traverse::run(mode, args.arg_paths, args.flag_recursive, &config)
}
