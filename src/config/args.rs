use std::path::PathBuf;

use docopt::Docopt;

use serde::Deserialize;

use crate::Action;

/// Docopt usage string.
const USAGE: &str = r#"
Spellcheck all your doc comments

Usage:
    cargo-spellcheck [(-v...|-q)] fix [--cfg=<cfg>] [--code=<code>] [--dev-comments] [--skip-readme] [--checkers=<checkers>] [[--recursive] <paths>... ]
    cargo-spellcheck [(-v...|-q)] reflow [--cfg=<cfg>] [--code=<code>] [--dev-comments] [--skip-readme] [[--recursive] <paths>... ]
    cargo-spellcheck [(-v...|-q)] config (--user|--stdout|--cfg=<cfg>) [--force]
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


/// Checker types to be derived from the stringly typed args.
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
    pub fn parse(mut argv_iter: impl Iterator<Item = String>) -> Result<Self, docopt::Error> {
        Docopt::new(USAGE).and_then(|d| {
            // if ends with file name `cargo-spellcheck`
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
                                if file_name.starts_with("cargo-spellcheck") && arg == "spellcheck" =>
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
            assert!(parse_args(commandline_to_iter(command))
                .map_err(|e| {
                    println!("Processing > {:?}", command);
                    e
                })
                .is_ok());
        }
    }

    #[test]
    fn action_extraction() {
        for (command, action) in SAMPLES.iter() {
            assert_eq!(
                parse_args(commandline_to_iter(command))
                    .expect("Parsing is assured by another unit test. qed")
                    .action(),
                *action
            );
        }
    }
}
