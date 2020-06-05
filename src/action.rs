use super::*;
use anyhow::{anyhow, Result};
use log::{debug, info, trace};
use std::convert::TryInto;
use std::fs::{self, OpenOptions};
use std::io::BufRead;
use std::io::Write;
use std::path::{Path, PathBuf};

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
    /// assumes suggestions are sorted by line number and column number and must be non overlapping
    fn correction<'s>(&self, path: PathBuf, suggestions: Vec<Suggestion<'s>>) -> Result<()> {
        let path = path.as_path().canonicalize().map_err(|e| { anyhow!("Failed to canonicalize {}", path.display() ).context(e) })?;
        let path = dbg!(path.as_path());
        trace!("Attempting to open {} as read", path.display());
        let ro = std::fs::OpenOptions::new()
            .read(true)
            .open(path)
            .map_err(|e| { anyhow!("Failed to open {}", path.display()).context(e) })?;

        let mut reader = std::io::BufReader::new(ro);

        const TEMPORARY: &'static str = ".spellcheck.tmp";

        let tmp = std::env::current_dir().expect("Must have cwd").join(TEMPORARY);
        // let tmp = tmp.canonicalize().map_err(|e| { anyhow!("Failed to canonicalize {}", tmp.display() ).context(e) })?;
        //trace!("Attempting to open {} as read", tmp.display());
        let wr = OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(&tmp)
            .map_err(|e| { anyhow!("Failed to open {}", path.display()).context(e) })?;

        let mut writer = std::io::BufWriter::with_capacity(1024, wr);

        let mut suggestions_it = suggestions.into_iter();
        let mut nxt: Option<Suggestion<'s>> = suggestions_it.next();

        for (line, content) in reader
            .lines()
            .enumerate()
            .map(|(lineno, content)| (lineno + 1, content))
        {
            trace!("Processing line {}", line);
            let mut remainder_column = 0usize;
            let content: String = content
                .map_err(|e| anyhow!("Line {} contains invalid utf8 characters", line).context(e) )?;

            if let Some(ref suggestion) = nxt {
                if !suggestion.span.covers_line(line) {
                    writer.write(content.as_bytes())?;
                    writer.write("\n".as_bytes())?;
                    continue
                }
            }

            while let Some(suggestion) = nxt.take() {
                trace!("Processing suggestion {}", suggestion);
                if let Some(replacement) = suggestion.replacements.first() {
                    let range: Range = suggestion
                        .span
                        .try_into()
                        .expect("There should be no multiline strings as of today");
                    // write prelude for this line between start or previous replacement
                    if range.start > remainder_column {
                        writer.write(content[remainder_column..range.start].as_bytes())?;
                    }
                    // write the replacement chunk
                    writer.write(replacement.as_bytes())?;

                    remainder_column = range.end;
                    nxt = suggestions_it.next();
                    if !suggestion.span.covers_line(line) {
                        // the last replacement may be the end of content
                        if remainder_column < content.len() {
                            // otherwise write all
                            // not that this also covers writing a line without any suggestions
                            writer.write(content[remainder_column..].as_bytes())?;
                        }
                        writer.write("\n".as_bytes())?;
                        // break the inner loop
                        break;
                        // } else {
                        // next suggestion covers same line
                    }
                } else {
                    debug!("Suggestion dues not contain any replacements, skipping");
                    nxt = suggestions_it.next();
                }
            }
        }
        writer.flush()?;

        fs::rename(tmp, path)?;

        Ok(())
    }

    // consume self, doing the same thing again would cause garbage file content
    pub fn write_changes_to_disk(
        &self,
        ratified_suggestions_per_path: SuggestionSet,
        config: &Config,
    ) -> Result<()> {
        info!("Writing changes back to disk");
        for (path, suggestions) in ratified_suggestions_per_path {
            self.correction(path, suggestions)?;
        }
        Ok(())
    }

    fn select_interactive<'s>(
        &self,
        suggestions_per_path: SuggestionSet<'s>,
        _config: &Config,
    ) -> Result<SuggestionSet<'s>> {
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

        // let _guard = ScopedRaw::new()?;

        let mut apply = SuggestionSet::new();

        trace!("Select the ones to actully use");

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
                    Direction::Backward => suggestions_it.next_back(), // FIXME @todo this is just plain wrong
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

                println!(">");

                let _guard = ScopedRaw::new()?;

                // read is blocking
                let event = match crossterm::event::read().map_err(|e| {
                    anyhow::anyhow!("Something unexpected happened on the CLI").context(e)
                })? {
                     Event::Key(event) => event,
                     sth => {
                        trace!("read() something other than a key: {:?}", sth);
                        break;
                     }
                };
                trace!("registered event: {:?}", &event);
                let KeyEvent { code, modifiers: _ } = event;


                match code {
                    KeyCode::Char('y') => {
                        apply.add(path.clone(), suggestion);
                    }
                    KeyCode::Char('n') => {}
                    KeyCode::Char('q') => return Ok(apply),
                    KeyCode::Char('a') => {
                        apply.add(path.clone(), suggestion);
                        apply.extend(path.clone(), suggestions_it.into_iter().map(|item| item.1));
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
        Ok(apply)
    }

    fn check(&self, suggestions_per_path: SuggestionSet, _config: &Config) -> Result<()> {
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
    pub fn run(self, suggestions_per_path: SuggestionSet, config: &Config) -> Result<()> {
        match self {
            Self::Fix => unimplemented!("Unsupervised fixing is not implemented just yet"),
            Self::Check => self.check(suggestions_per_path, config)?,
            Self::Interactive => {
                let selected = self.select_interactive(suggestions_per_path, config)?;
                self.write_changes_to_disk(selected, config)?;
            }
        }
        Ok(())
    }
}




struct ScopedRaw {
    _dummy: u8
}

impl ScopedRaw {
    fn new() -> Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok( Self {
            _dummy: 0u8
        })
    }
}


impl Drop for ScopedRaw {
    fn drop(&mut self) {
        crossterm::terminal::disable_raw_mode();
    }
}

