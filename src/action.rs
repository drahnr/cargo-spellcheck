use super::*;
use anyhow::{anyhow, Error, Result};
use log::{debug, info, trace};
use std::convert::{TryFrom, TryInto};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, Read, Write};

use std::path::PathBuf;

#[derive(Debug, Clone)]
struct BandAid {
    /// a span, where the first line has index 1, columns are base 1 too
    pub span: Span,
    /// replacement text for the given span
    pub replacement: String,
}

impl<'s> TryFrom<Suggestion<'s>> for BandAid {
    type Error = Error;
    fn try_from(suggestion: Suggestion<'s>) -> Result<Self> {
        let literal_file_span = suggestion.span;
        trace!(
            "proc_macro literal span of doc comment: ({},{})..({},{})",
            literal_file_span.start.line,
            literal_file_span.start.column,
            literal_file_span.end.line,
            literal_file_span.end.column
        );

        if let Some(replacement) = suggestion.replacements.into_iter().next() {
            let mut span = suggestion.span;
            // @todo this is a hack and should be documented better
            // @todo not sure why the offset of two is necessary
            // @todo but it works consistently
            let doc_comment_to_file_offset = 2;
            span.start.column += doc_comment_to_file_offset;
            span.end.column += doc_comment_to_file_offset;
            Ok(Self {
                span,
                replacement: replacement.to_owned(),
            })
        } else {
            Err(anyhow!("Does not contain any replacements"))
        }
    }
}

/// correct all lines
/// `bandaids` are the fixes to be applied to the lines
///
/// Note that `Lines` as created by `(x as BufLines).lines()` does
/// not preserve trailing newlines, so either the iterator
/// needs to be modified to yield an extra (i.e. with `.chain("".to_owned())`)
/// or a manual newlines has to be written to the `sink`.
fn correct_lines<'s>(
    mut bandaids: impl Iterator<Item = BandAid>,
    source: impl Iterator<Item = (usize, String)>,
    mut sink: impl Write,
) -> Result<()> {
    let mut nxt: Option<BandAid> = bandaids.next();
    for (line_number, content) in source {
        trace!("Processing line {}", line_number);
        let mut remainder_column = 0usize;
        // let content: String = content.map_err(|e| {
        //     anyhow!("Line {} contains invalid utf8 characters", line_number).context(e)
        // })?;

        if nxt.is_none() {
            // no candidates remaining, just keep going
            sink.write(content.as_bytes())?;
            sink.write("\n".as_bytes())?;
            continue;
        }

        if let Some(ref bandaid) = nxt {
            if !bandaid.span.covers_line(line_number) {
                sink.write(content.as_bytes())?;
                sink.write("\n".as_bytes())?;
                continue;
            }
        }

        while let Some(bandaid) = nxt.take() {
            trace!("Applying next bandaid {:?}", bandaid);
            trace!("where line {} is: >{}<", line_number, content);
            let range: Range = bandaid
                .span
                .try_into()
                .expect("There should be no multiline strings as of today");
            // write prelude for this line between start or previous replacement
            if range.start > remainder_column {
                sink.write(content[remainder_column..range.start].as_bytes())?;
            }
            // write the replacement chunk
            sink.write(bandaid.replacement.as_bytes())?;

            remainder_column = range.end;
            nxt = bandaids.next();
            let complet_current_line = if let Some(ref bandaid) = nxt {
                // if `nxt` is also targeting the current line, don't complete the line
                !bandaid.span.covers_line(line_number)
            } else {
                true
            };
            if complet_current_line {
                // the last replacement may be the end of content
                if remainder_column < content.len() {
                    debug!(
                        "line {} len is {}, and remainder column is {}",
                        line_number,
                        content.len(),
                        remainder_column
                    );
                    // otherwise write all
                    // not that this also covers writing a line without any suggestions
                    sink.write(content[remainder_column..].as_bytes())?;
                } else {
                    debug!(
                        "line {} len is {}, and remainder column is {}",
                        line_number,
                        content.len(),
                        remainder_column
                    );
                }
                sink.write("\n".as_bytes())?;
                // break the inner loop
                break;
                // } else {
                // next suggestion covers same line
            }
        }
    }
    Ok(())
}

/// Mode in which `cargo-spellcheck` operates
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Action {
    /// Fix issues without interaction if there is sufficient information
    Fix,
    /// Only show errors
    Check,
    /// Interactively choose from __candidates__ provided, similar to `git add -p` .
    Interactive,
}

impl Action {
    /// assumes suggestions are sorted by line number and column number and must be non overlapping
    fn correction<'s>(&self, path: PathBuf, suggestions: Vec<Suggestion<'s>>) -> Result<()> {
        let path = path
            .as_path()
            .canonicalize()
            .map_err(|e| anyhow!("Failed to canonicalize {}", path.display()).context(e))?;
        let path = dbg!(path.as_path());
        trace!("Attempting to open {} as read", path.display());
        let ro = std::fs::OpenOptions::new()
            .read(true)
            .open(path)
            .map_err(|e| anyhow!("Failed to open {}", path.display()).context(e))?;

        let mut reader = std::io::BufReader::new(ro);

        const TEMPORARY: &'static str = ".spellcheck.tmp";

        let tmp = std::env::current_dir()
            .expect("Must have cwd")
            .join(TEMPORARY);
        // let tmp = tmp.canonicalize().map_err(|e| { anyhow!("Failed to canonicalize {}", tmp.display() ).context(e) })?;
        //trace!("Attempting to open {} as read", tmp.display());
        let wr = OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(&tmp)
            .map_err(|e| anyhow!("Failed to open {}", path.display()).context(e))?;

        let mut writer = std::io::BufWriter::with_capacity(1024, wr);

        correct_lines(
            suggestions
                .into_iter()
                .filter_map(|suggestion: Suggestion| -> Option<BandAid> {
                    BandAid::try_from(suggestion)
                        .or_else(|e| {
                            warn!("Suggestion does not contain any replacements");
                            Err(e)
                        })
                        .ok()
                }),
            (&mut reader)
                .lines()
                .filter_map(|line| line.ok())
                .enumerate()
                .map(|(lineno, content)| (lineno + 1, content)),
            &mut writer,
        )?;

        writer.flush()?;

        fs::rename(tmp, path)?;

        Ok(())
    }

    // consume self, doing the same thing again would cause garbage file content
    pub fn write_changes_to_disk(
        &self,
        ratified_suggestions_per_path: SuggestionSet,
        _config: &Config,
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

/// Helper struct to assure we leave the terminals raw mode
struct ScopedRaw {
    _dummy: u8,
}

impl ScopedRaw {
    fn new() -> Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self { _dummy: 0u8 })
    }
}

impl Drop for ScopedRaw {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &'static str = r#"
I like unicorns every second Mondays.

"#;

    const CORRECTED: &'static str = r#"
I like banana icecream every third day.

"#;

    #[test]
    fn replace_unicorns() {
        let _ = env_logger::Builder::new()
            .filter(None, log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        let mut sink: Vec<u8> = Vec::with_capacity(1024);
        let bandaids = vec![
            BandAid {
                span: (2usize, 7..15).try_into().unwrap(),
                replacement: "banana icecream".to_owned(),
            },
            BandAid {
                span: (2usize, 22..28).try_into().unwrap(),
                replacement: "third".to_owned(),
            },
            BandAid {
                span: (2usize, 29..36).try_into().unwrap(),
                replacement: "day".to_owned(),
            },
        ];

        let lines = TEXT
            .lines()
            .map(|line| line.to_owned())
            .enumerate()
            .map(|(lineno, content)| (lineno + 1, content));

        correct_lines(bandaids.into_iter(), lines, &mut sink).expect("should be able to");

        assert_eq!(String::from_utf8_lossy(sink.as_slice()), CORRECTED);
    }
}
