mod config;
mod documentation;
mod literalset;
mod span;

mod checker;
mod extractor;
mod suggestion;

pub use self::documentation::*;
pub use self::literalset::*;
pub use self::span::*;
pub use self::suggestion::*;

use docopt::Docopt;
use log::{debug, info, trace, warn};
use serde::Deserialize;

use std::path::PathBuf;

const USAGE: &str = r#"
Spellcheck all your doc comments

Usage:
  cargo spellcheck check [[--recursive] <paths>.. ]
  cargo-spellcheck check [[--recursive] <paths>.. ]
  cargo spellcheck fix [[--recursive] <paths>.. ]
  cargo-spellcheck fix [[--recursive] <paths>.. ]
  cargo spellcheck [(--fix|--interactive)] [[--recursive] <paths>.. ]
  cargo-spellcheck [(--fix|--interactive)] [[--recursive] <paths>.. ]

Options:
  -h --help           Show this screen.
  --fix               Synonym to running the `fix` subcommand.
  -i --interactive    Interactively apply spelling and grammer fixes.
  -r --recursive      If a path is provided, if recursion into subdirectories is desired.
"#;

#[derive(Debug, Deserialize)]
struct Args {
    flag_recursive: bool,
    arg_paths: Vec<PathBuf>,
    flag_fix: bool,
    flag_interactive: bool,
    cmd_fix: bool,
    cmd_check: bool,
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

    let args: Args = Docopt::new(USAGE)
        .and_then(|d| d.deserialize())
        .unwrap_or_else(|e| e.exit());

    let mode = if args.cmd_fix || args.flag_fix {
        Mode::Fix
    } else if args.flag_interactive {
        Mode::Interactive
    } else {
        // check
        Mode::Check
    };

    trace!("Executing: {:?}", mode);
    extractor::run(mode, args.arg_paths, args.flag_recursive)
}
