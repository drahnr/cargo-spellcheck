mod action;
mod checker;
mod config;
mod documentation;
mod span;
mod suggestion;
mod traverse;

pub use self::action::*;
pub use self::config::{Config, HunspellConfig, LanguageToolConfig};
pub use self::documentation::*;
pub use self::span::*;
pub use self::suggestion::*;

use docopt::Docopt;

use log::{info, trace, warn};
use serde::Deserialize;
use signal_hook::{iterator, SIGINT, SIGQUIT, SIGTERM};

use std::path::PathBuf;

const USAGE: &str = r#"
Spellcheck all your doc comments

Usage:
    cargo-spellcheck [(-v...|-q)] check [--cfg=<cfg>] [--checkers=<checkers>] [[--recursive] <paths>... ]
    cargo-spellcheck [(-v...|-q)] fix [--cfg=<cfg>] [--interactive] [--checkers=<checkers>] [[--recursive] <paths>... ]
    cargo-spellcheck [(-v...|-q)] config (--user|--stdout|--cfg=<cfg>) [--force]
    cargo-spellcheck [(-v...|-q)] [--cfg=<cfg>] [--fix [--interactive]] [--checkers=<checkers>] [[--recursive] <paths>... ]
    cargo-spellcheck --help
    cargo-spellcheck --version

Options:
  -h --help               Show this screen.
  --version               Print the version and exit.

  --fix                   Synonym to running the `fix` subcommand.
  -i --interactive        Interactively apply spelling and grammer fixes.
  -r --recursive          If a path is provided, if recursion into subdirectories is desired.
  --checkers=<checkers>   Calculate the intersection between
                          configured by config file and the ones provided on commandline.
  -f --force              Overwrite any existing configuration file. [default=false]
  -c --cfg=<cfg>          Use a non default configuration file.
                          Passing a directory will attempt to open `cargo_spellcheck.toml` in that directory.
  --user                  Write the configuration file to the default user configuration directory.
  --stdout                Print the configuration file to stdout and exit.
  -v --verbose            Verbosity level.
  -q --quiet              Silences all printed messages. Overrules `-v`.

"#;

#[derive(Debug, Deserialize, Default)]
struct Args {
    arg_paths: Vec<PathBuf>,
    flag_fix: bool,
    flag_interactive: bool,
    flag_recursive: bool,
    flag_verbose: usize,
    flag_quiet: bool,
    flag_version: bool,
    flag_help: bool,
    flag_checkers: Option<String>,
    flag_cfg: Option<PathBuf>,
    flag_force: bool,
    flag_user: bool,
    flag_stdout: bool,
    cmd_fix: bool,
    cmd_check: bool,
    cmd_config: bool,
}

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

fn main() -> anyhow::Result<()> {
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
        return Ok(());
    }

    if args.flag_help {
        println!("{}", USAGE);
        return Ok(());
    }

    let on_exit = || match crossterm::terminal::disable_raw_mode() {
        Ok(_) => std::process::exit(0),
        Err(_) => std::process::exit(1),
    };

    let signals = iterator::Signals::new(vec![SIGTERM, SIGINT, SIGQUIT])?;
    std::thread::spawn(move || {
        for s in signals.forever() {
            match s {
                SIGTERM => on_exit(),
                SIGINT => on_exit(),
                SIGQUIT => on_exit(),
                _ => unreachable!(),
            }
        }
    });

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
            return Ok(());
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
        return Ok(());
    } else {
        trace!("Not configuration sub command");
    }

    let (explicit_cfg, config_path) = match args.flag_cfg.as_ref() {
        Some(path) => (true, path.to_owned()),
        _ => (false, Config::default_path()?),
    };
    let mut config = match Config::load_from(&config_path) {
        Ok(config) => config,
        Err(e) => {
            if explicit_cfg {
                return Err(anyhow::anyhow!(
                    "Explicitly given config file does not exist"
                ));
            } else {
                warn!(
                    "Loading configuration from {}, due to: {}",
                    config_path.display(),
                    e
                );
                Config::default()
            }
        }
    };

    checkers(&mut config);

    // extract operation mode
    let action = if args.flag_interactive {
        Action::Interactive
    } else if args.cmd_fix || args.flag_fix {
        Action::Fix
    } else {
        // check
        Action::Check
    };

    trace!("Executing: {:?} with {:?}", action, &config);

    let combined = traverse::extract(args.arg_paths, args.flag_recursive, &config)?;

    let suggestion_set = checker::check(&combined, &config)?;

    action.run(suggestion_set, &config)
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
            "cargo spellcheck --fix --interactive",
            "cargo spellcheck fix",
            "cargo spellcheck fix --interactive",
            "cargo spellcheck fix --interactive -r file.rs",
            "cargo spellcheck -q fix --interactive Cargo.toml",
            "cargo spellcheck -v fix --interactive Cargo.toml",
            "cargo-spellcheck",
            "cargo-spellcheck -vvvv",
            "cargo-spellcheck --fix",
            "cargo-spellcheck --fix --interactive",
            "cargo-spellcheck fix",
            "cargo-spellcheck fix --interactive",
            "cargo-spellcheck fix --interactive -r file.rs",
            "cargo-spellcheck -q fix --interactive Cargo.toml",
            "cargo spellcheck -v fix --interactive Cargo.toml",
        ];
        for command in commands {
            assert!(parse_args(commandline_to_iter(command)).is_ok());
        }
    }
}
