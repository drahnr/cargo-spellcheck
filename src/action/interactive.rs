//! Interactive picking of replacements, contained in a suggestion.
//!
//! The result of that pick is a bandaid.

use super::*;

use crossterm;

use crossterm::{
    cursor,
    event::{Event, KeyCode, KeyEvent},
    style::{style, Attribute, Color, ContentStyle, Print, PrintStyledContent, StyledContent},
    terminal, QueueableCommand,
};

use std::io::stdout;
use std::path::Path;

const HELP: &'static str = r##"y - apply this suggestion
n - do not apply the suggested correction
q - quit; do not stage this hunk or any of the remaining ones
d - do not apply this suggestion and skip the rest of the file
g - select a suggestion to go to
j - leave this hunk undecided, see next undecided hunk
J - leave this hunk undecided, see next hunk
e - manually edit the current hunk
? - print help




"##;

/// Helper strict to assure we leave the terminals raw mode
struct ScopedRaw;

impl ScopedRaw {
    fn new() -> Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for ScopedRaw {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// In which direction we should progress
#[derive(Debug, Clone, Copy)]
enum Direction {
    Forward,
    Backward,
}

/// The user picked something. This is the pick representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Pick {
    Replacement(BandAid),
    Skip,
    Previous,
    Help,
    SkipFile,
    Quit,
}

/// The selection of used suggestion replacements
#[derive(Debug, Clone, Default)]
pub struct UserPicked {
    pub bandaids: indexmap::IndexMap<PathBuf, Vec<BandAid>>,
}

impl UserPicked {
    /// Count the number of suggestions accress file in total
    pub fn count(&self) -> usize {
        self.bandaids.iter().map(|(_path, vec)| vec.len()).sum()
    }

    /// Apply a single bandaid.
    fn add_bandaid<'u>(&mut self, path: &Path, fix: BandAid) {
        self.bandaids
            .entry(path.to_owned())
            .or_insert_with(|| Vec::with_capacity(10))
            .push(fix);
    }

    /// Apply multiple bandaids.
    fn add_bandaids<I>(&mut self, path: &Path, fixes: I)
    where
        I: IntoIterator<Item = BandAid>,
    {
        let iter = fixes.into_iter();
        self.bandaids
            .entry(path.to_owned())
            .or_insert_with(|| Vec::with_capacity(iter.size_hint().0))
            .extend(iter);
    }

    /// only print the list of replacements to the user
    // initial thougth was to show a horizontal list of replacements, navigate left/ right
    // by using the arrow keys
    // .. suggestion0 [suggestion1] suggestion2 suggestion3 ..
    // arrow left
    // .. suggestion1 [suggestion2] suggestion3 suggestion4 ..
    // but now it's only a very simple list for now
    fn print_replacements_list(&self, suggestion: &Suggestion, active_idx: usize) -> Result<()> {
        let mut stdout = stdout();

        let tick = ContentStyle::new()
            .foreground(Color::Green)
            .attribute(Attribute::Bold);

        let highlight = ContentStyle::new()
            .background(Color::Black)
            .foreground(Color::Green)
            .attribute(Attribute::Bold);

        let others = ContentStyle::new()
            .background(Color::Black)
            .foreground(Color::Blue);

        // render all replacements in a vertical list

        stdout.queue(cursor::SavePosition).unwrap();
        let _ = stdout.flush();

        suggestion
            .replacements
            .iter()
            .enumerate()
            .for_each(|(idx, replacement)| {
                let idx = idx as u16;
                if idx != active_idx as u16 {
                    // @todo figure out a way to deal with those errors better
                    stdout
                        // .queue(cursor::MoveTo(start.0 + idx, start.1)).unwrap()
                        .queue(cursor::MoveUp(1))
                        .unwrap()
                        .queue(terminal::Clear(terminal::ClearType::CurrentLine))
                        .unwrap()
                        .queue(cursor::MoveToColumn(4))
                        .unwrap()
                        .queue(PrintStyledContent(StyledContent::new(
                            others.clone(),
                            replacement,
                        )))
                        .unwrap();
                } else {
                    stdout
                        // .queue(cursor::MoveTo(start.0 + idx, start.1)).unwrap()
                        .queue(cursor::MoveUp(1))
                        .unwrap()
                        .queue(terminal::Clear(terminal::ClearType::CurrentLine))
                        .unwrap()
                        .queue(cursor::MoveToColumn(2))
                        .unwrap()
                        .queue(PrintStyledContent(StyledContent::new(tick.clone(), '»')))
                        .unwrap()
                        .queue(cursor::MoveToColumn(4))
                        .unwrap()
                        .queue(PrintStyledContent(StyledContent::new(
                            highlight.clone(),
                            replacement,
                        )))
                        .unwrap();
                }
            });

        stdout.queue(cursor::RestorePosition).unwrap();

        let _ = stdout.flush();
        Ok(())
    }

    /// Wait for user input and process it into a `Pick` enum
    fn user_input<'i>(
        &self,
        suggestion: &'i Suggestion,
        running_idx: (usize, usize),
    ) -> Result<Pick> {
        {
            let _guard = ScopedRaw::new();

            let boring = ContentStyle::new()
                .foreground(Color::Blue)
                .attribute(Attribute::Bold);

            let question = format!(
                "({nth}/{of_n}) Apply this suggestion [y,n,q,a,d,j,e,?]?",
                nth = running_idx.0 + 1,
                of_n = running_idx.1
            );

            // a new suggestion, so prepare for the number of items that are visible
            // and also overwrite the last lines of the regular print which would
            // already contain the suggestions
            stdout()
                .queue(cursor::Hide)
                .unwrap()
                .queue(cursor::MoveToColumn(0))
                .unwrap()
                .queue(cursor::MoveUp(5))
                .unwrap()
                .queue(cursor::MoveToColumn(0))
                .unwrap()
                .queue(terminal::Clear(terminal::ClearType::CurrentLine))
                .unwrap()
                .queue(cursor::MoveDown(1))
                .unwrap()
                .queue(cursor::MoveToColumn(0))
                .unwrap()
                .queue(terminal::Clear(terminal::ClearType::CurrentLine))
                .unwrap()
                .queue(cursor::MoveToColumn(0))
                .unwrap()
                .queue(PrintStyledContent(StyledContent::new(boring, question)))
                .unwrap()
                .queue(cursor::MoveToColumn(0))
                .unwrap()
                .queue(cursor::MoveDown(1))
                .unwrap()
                .queue(terminal::Clear(terminal::ClearType::CurrentLine))
                .unwrap()
                .queue(cursor::MoveDown(1))
                .unwrap()
                .queue(terminal::Clear(terminal::ClearType::CurrentLine))
                .unwrap() // @todo deal with error conversion
                .queue(terminal::ScrollUp(suggestion.replacements.len() as u16))
                .unwrap();
        }

        // which index to show as highlighted
        let mut pick_idx = 0usize;
        loop {
            let guard = ScopedRaw::new();

            self.print_replacements_list(suggestion, pick_idx)?;

            let event = match crossterm::event::read()
                .map_err(|e| anyhow::anyhow!("Something unexpected happened on the CLI: {}", e))?
            {
                Event::Key(event) => event,
                sth => {
                    trace!("read() something other than a key: {:?}", sth);
                    break;
                }
            };
            drop(guard);
            // print normally again

            trace!("registered event: {:?}", &event);
            let KeyEvent { code, modifiers: _ } = event;

            let n_replacements = suggestion.replacements.len();

            match code {
                KeyCode::Left | KeyCode::Up => {
                    pick_idx = (pick_idx + 1).rem_euclid(n_replacements);
                }
                KeyCode::Down | KeyCode::Right => {
                    pick_idx = (pick_idx + n_replacements - 1).rem_euclid(n_replacements);
                }
                KeyCode::Enter | KeyCode::Char('y') => {
                    // current: must succeed, suggestions with replacements
                    // are supposed to considered earlier
                    let bandaid: BandAid = ((suggestion, pick_idx)).try_into()?;
                    // @todo handle interactive intput for those where there are no suggestions
                    return Ok(Pick::Replacement(bandaid));
                }
                KeyCode::Char('n') => return Ok(Pick::Skip),
                KeyCode::Char('j') => return Ok(Pick::Previous),
                KeyCode::Char('q') | KeyCode::Esc => return Ok(Pick::Quit),
                KeyCode::Char('d') => return Ok(Pick::SkipFile),
                KeyCode::Char('e') => unimplemented!("Manual editing is a TODO"),
                KeyCode::Char('?') => return Ok(Pick::Help),
                x => {
                    trace!("Unexpected input {:?}", x);
                }
            }
        }
        unreachable!("Unexpected return when dealing with user input")
    }

    pub(super) fn select_interactive<'s>(
        suggestions_per_path: SuggestionSet<'s>,
        _config: &Config,
    ) -> Result<Self> {
        let mut picked = UserPicked::default();

        trace!("Select the ones to actully use");

        for (path, suggestions) in suggestions_per_path {
            let count = suggestions.len();
            println!("Path is {} and has {}", path.display(), count);

            // @todo juck, uggly
            let mut suggestions_it = suggestions.clone().into_iter().enumerate();

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
                if suggestion.replacements.is_empty() {
                    trace!("Suggestion did not contain a replacement, skip");
                    continue;
                }
                println!("{}", suggestion);

                let mut pick = picked.user_input(&suggestion, (idx, count))?;
                while pick == Pick::Help {
                    println!("{}", HELP);
                    pick = picked.user_input(&suggestion, (idx, count))?;
                }
                match pick {
                    Pick::Quit => {
                        unimplemented!("Quit properly and cleanly");
                    }
                    Pick::SkipFile => break, // break the inner loop
                    Pick::Skip => {}
                    Pick::Previous => {
                        unimplemented!("Requires a iterator which works bidrectionally")
                    }
                    Pick::Help => unreachable!(),
                    Pick::Replacement(bandaid) => {
                        picked.add_bandaid(&path, bandaid);
                    }
                };

                direction = Direction::Forward;
            }
        }
        Ok(picked)
    }
}
