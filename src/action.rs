use super::*;
use anyhow::{anyhow, Result};

/// Mode in which `cargo-spellcheck` operates
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Action {
    /// Fix issues without interaction if there is sufficient information
    Fix,
    /// Only show errors
    Check,
    /// Interactively choose from __candidates__ provided, simliar to `git add -p` .
    Interactive,
}

impl Action {
    fn fix_interactive(&self, suggestions_per_path: SuggestionSet, config: &Config) -> Result<()> {
        // show a horizontal list of replacements, navigate left/ right by using the arrow keys
        // .. suggestion0 [suggestion1] suggestion2 suggestion3 ..
        // arrow left
        // .. suggestion1 [suggestion2] suggestion3 suggestion4 ..
        use crossterm;

        use crossterm::{
            cursor, event::Event, event::KeyCode, event::KeyEvent, style::Print, QueueableCommand,
            Result,
        };
        use std::io::stdout;

        const HELP: &'static str = r##"y - apply this suggestion
n - do not apply the suggested correction
q - quit; do not stage this hunk or any of the remaining ones
a - stage this hunk and all later hunks in the file
d - do not apply this suggestion and skip the rest of the file
g - select a suggestion to go to
j - leave this hunk undecided, see next undecided hunk
J - leave this hunk undecided, see next hunk
e - manually edit the current hunk
? - print help
"##;

        let _stdout = stdout();

        let mut apply = indexmap::IndexSet::<Suggestion<'_>>::new();

        for (path, suggestions) in suggestions_per_path {
            let count = suggestions.len();
            println!("Path is {} and has {}", path.display(), count);

            // @todo juck, uggly
            let mut suggestions_it = suggestions.clone().into_iter().enumerate();

            #[derive(Debug, Clone, Copy)]
            enum Direction {
                Forward,
                Backward,
            }
            let mut direction = Direction::Forward;
            loop {
                let opt: Option<(usize, Suggestion)> = match direction {
                    Direction::Forward => suggestions_it.next(),
                    Direction::Backward => suggestions_it.next_back(),
                };

                trace!("next() ---> {:?}", &opt);

                if opt.is_none() {
                    match direction {
                        Direction::Forward => {
                            trace!("completed file, continue to next");
                            break; // we completed this file, move on to the next
                        }
                        Direction::Backward => {
                            trace!("went back, now back at the beginning");
                            suggestions_it = suggestions.clone().into_iter().enumerate();
                            continue;
                        } // go to the start
                    }
                }
                let (idx, suggestion) = opt.expect("Must be X");

                println!("{}", suggestion);

                println!(
                    "({nth}/{of_n}) Apply this suggestion [y,n,q,a,d,j,e,?]?",
                    nth = idx,
                    of_n = count
                );

                // read is blocking
                let event = if let Event::Key(event) = crossterm::event::read().map_err(|e| {
                    anyhow::anyhow!("Something unexpected happened on the CLI: {}", e)
                })? {
                    event
                } else {
                    trace!("read() something othe than a key event an error");
                    break;
                };
                let KeyEvent { code, modifiers: _ } = event;

                match code {
                    KeyCode::Char('y') => {
                        apply.insert(suggestion);
                    }
                    KeyCode::Char('n') => {}
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('a') => {
                        for (_, suggestion) in suggestions_it {
                            apply.insert(suggestion);
                        }
                        break;
                    }
                    KeyCode::Char('d') => break,
                    KeyCode::Char('j') => {
                        direction = Direction::Backward;
                        continue;
                    }
                    KeyCode::Char('e') => unimplemented!("Manual editing"),
                    KeyCode::Char('?') => {
                        println!("{}", HELP);
                    }
                    x => {
                        trace!("Unexpected input {:?}", x);
                    }
                }
                direction = Direction::Forward;
            }
        }
        Ok(())
    }

    fn check(&self, suggestions_per_path: SuggestionSet, config: &Config) -> Result<()> {
        let mut count = 0usize;
        for (_path, suggestions) in suggestions_per_path {
            count += suggestions.len();
            for suggestion in suggestions {
                eprintln!("{}", suggestion);
            }
        }
        if count > 0 {
            Err(anyhow::anyhow!(
                "Found {} potential spelling mistakes",
                count
            ))
        } else {
            Ok(())
        }
    }

    /// Run the requested action.
    pub fn run(&self, suggestions_per_path: SuggestionSet, config: &Config) -> Result<()> {
        match self {
            Self::Fix => unimplemented!("Unsupervised fixing is not implemented just yet"),
            Self::Check => self.check(suggestions_per_path, config)?,
            Self::Interactive => self.fix_interactive(suggestions_per_path, config)?,
        }
        Ok(())
    }
}
