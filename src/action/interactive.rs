//! Interactive picking of replacements, contained in a suggestion.
//!
//! The result of that pick is a bandaid.

use super::*;

use crossterm;

use crossterm::{
    cursor,
    event::{Event, KeyCode, KeyEvent, KeyModifiers},
    style::{Attribute, Color, ContentStyle, PrintStyledContent, StyledContent},
    terminal, QueueableCommand,
};

use std::convert::TryFrom;
use std::io::stdout;

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
pub struct ScopedRaw;

impl ScopedRaw {
    /// Enter raw terminal mode.
    ///
    /// Must be left before using `log::info!(..)` or any
    /// other printing macros or functions.
    fn new() -> Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self)
    }

    /// Helper to restore the previous terminal state.
    ///
    /// Also called on `drop`.
    pub fn restore_terminal() -> Result<()> {
        stdout().queue(crossterm::cursor::Show)?;
        crossterm::terminal::disable_raw_mode()?;
        stdout().flush().map_err(|e| anyhow::anyhow!(e))
    }
}

impl Drop for ScopedRaw {
    fn drop(&mut self) {
        let _ = Self::restore_terminal();
    }
}

/// In which direction we should progress.
#[derive(Debug, Clone, Copy)]
enum Direction {
    /// In order.
    Forward,
    /// Reverse order from the current position.
    #[allow(unused)]
    Backward,
}

/// The user picked something. This is the pick representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum UserSelection {
    /// This `FirstAidKit` is going to be applied.
    Replacement(FirstAidKit),
    /// Skip this suggestion and move on to the next suggestion.
    Skip,
    /// Jump to the previous suggestion.
    Previous,
    /// Print the help message and exit.
    Help,
    /// Skip the remaining fixes for the current file.
    SkipFile,
    /// Stop execution.
    Abort,
    /// continue as if whatever returned this was never called.
    Nop,
}

/// Statefulness for the selection process
struct State<'s, 't>
where
    't: 's,
{
    /// Which suggestion is operated upon.
    pub suggestion: &'s Suggestion<'t>,
    /// The content the user provided for the suggestion, if any.
    pub custom_replacement: String,
    pub cursor_offset: u16,
    /// Which index to show as highlighted.
    pub pick_idx: usize,
    /// Total number of pickable slots.
    pub n_items: usize,
}

impl<'s, 't> From<&'s Suggestion<'t>> for State<'s, 't> {
    fn from(suggestion: &'s Suggestion<'t>) -> Self {
        Self {
            suggestion,
            custom_replacement: String::new(),
            cursor_offset: 0,
            pick_idx: 0usize,
            // all items provided by the checkers plus the user provided
            n_items: suggestion.replacements.len() + 1,
        }
    }
}

impl<'s, 't> State<'s, 't>
where
    't: 's,
{
    /// Selects the next line.
    pub fn select_next(&mut self) {
        self.pick_idx = (self.pick_idx + 1).rem_euclid(self.n_items);
    }

    /// Selects the previous line.
    pub fn select_previous(&mut self) {
        self.pick_idx = (self.pick_idx + self.n_items - 1).rem_euclid(self.n_items);
    }

    /// Select the custom line, which is by definition the
    /// last selectable.
    pub fn select_custom(&mut self) {
        self.pick_idx = self.n_items - 1;
    }

    /// Checks if the currently selected line is the custom entry.
    pub fn is_custom_entry(&self) -> bool {
        self.pick_idx + 1 == self.n_items
    }

    /// Convert the replacement to a `FirstAidKit`.
    pub fn to_first_aid_kit(&self) -> FirstAidKit {
        if self.is_custom_entry() {
            FirstAidKit::try_from((self.custom_replacement.clone(), &self.suggestion.span))
        } else {
            let replacement = self
                .suggestion
                .replacements
                .get(self.pick_idx)
                .expect("User pick index is out of bounds");
            FirstAidKit::load_from(
                &self.suggestion.chunk,
                self.suggestion.span,
                replacement.to_owned(),
            )
        }
        .expect("Extracting `Bandaid`s from `State` must not fail. qed")
    }
}

/// The selection of used suggestion replacements.
#[derive(Debug, Clone, Default)]
pub struct UserPicked {
    /// Associates the bandaids to a content origin, or path respectively.
    pub bandaids: indexmap::IndexMap<ContentOrigin, Vec<BandAid>>,
}

impl UserPicked {
    /// Count the number of suggestions accross all files in total.
    pub fn total_count(&self) -> usize {
        self.bandaids.iter().map(|(_origin, vec)| vec.len()).sum()
    }

    /// Apply a `FirstAidKit` consisting of multiple BandAids
    pub fn add_first_aid_kit(&mut self, origin: &ContentOrigin, kit: FirstAidKit) {
        self.bandaids
            .entry(origin.clone())
            .or_insert_with(|| Vec::with_capacity(10))
            .extend(kit.bandaids);
    }

    /// Apply multiple bandaids.
    /// TODO: Still needed? Re-implement for FirstAidKit?
    #[allow(unused)]
    fn add_bandaids<I>(&mut self, origin: &ContentOrigin, fixes: I)
    where
        I: IntoIterator<Item = BandAid>,
    {
        let iter = fixes.into_iter();
        self.bandaids
            .entry(origin.clone())
            .or_insert_with(|| Vec::with_capacity(iter.size_hint().0))
            .extend(iter);
    }

    /// Provide a replacement that was not provided by the backend
    fn enter_custom_replacement(
        &self,
        state: &mut State,
        event: KeyEvent,
    ) -> Result<UserSelection> {
        let KeyEvent { code, modifiers } = event;

        let length = state.custom_replacement.len() as u16;
        match code {
            KeyCode::Left => state.cursor_offset = state.cursor_offset.saturating_sub(1),
            KeyCode::Right => state.cursor_offset = (state.cursor_offset + 1).min(length),
            KeyCode::Up => {
                state.cursor_offset = length;
                state.select_next();
            }
            KeyCode::Down => {
                state.cursor_offset = length;
                state.select_previous();
            }
            KeyCode::Backspace => {
                if state.cursor_offset > 0 {
                    state.cursor_offset -= 1;
                    state
                        .custom_replacement
                        .remove(state.cursor_offset as usize);
                }
            }
            KeyCode::Enter => {
                let kit = FirstAidKit::try_from((
                    state.custom_replacement.clone(),
                    &state.suggestion.span,
                ))?;
                return Ok(UserSelection::Replacement(kit));
            }
            KeyCode::Esc => return Ok(UserSelection::Abort),
            KeyCode::Char('c') if modifiers == KeyModifiers::CONTROL => {
                return Ok(UserSelection::Abort);
            }
            KeyCode::Char(c) => {
                state
                    .custom_replacement
                    .insert(state.cursor_offset as usize, c);
                state.cursor_offset += 1;
            }
            _ => {}
        }

        Ok(UserSelection::Nop)
    }

    /// Only print the list of replacements to the user.
    // initial thougth was to show a horizontal list of replacements, navigate left/ right
    // by using the arrow keys
    // .. suggestion0 [suggestion1] suggestion2 suggestion3 ..
    // arrow left
    // .. suggestion1 [suggestion2] suggestion3 suggestion4 ..
    // but now it's only a very simple list for now
    fn print_replacements_list(&self, state: &mut State) -> Result<()> {
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

        let custom = ContentStyle::new()
            .background(Color::Black)
            .foreground(Color::Yellow);

        // render all replacements in a vertical list

        stdout.queue(cursor::SavePosition).unwrap();
        let _ = stdout.flush();

        let active_idx = state.pick_idx;

        let custom_content = if state.custom_replacement.is_empty() {
            "..."
        } else {
            state.custom_replacement.as_str()
        };
        if state.n_items != active_idx + 1 {
            stdout
                .queue(cursor::MoveUp(1))
                .unwrap()
                .queue(terminal::Clear(terminal::ClearType::CurrentLine))
                .unwrap()
                .queue(cursor::MoveToColumn(4))
                .unwrap()
                .queue(PrintStyledContent(StyledContent::new(
                    custom,
                    custom_content,
                )))
                .unwrap();
        } else {
            stdout
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
                    custom,
                    custom_content,
                )))
                .unwrap();
        }
        let _ = stdout.flush();

        state
            .suggestion
            .replacements
            .iter()
            .enumerate()
            .for_each(|(idx, replacement)| {
                let idx = idx as u16;
                if idx != active_idx as u16 {
                    // TODO figure out a way to deal with those errors better
                    stdout
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

    /// Wait for user input and process it into a `UserSelection` enum.
    fn user_input(&self, state: &mut State, running_idx: (usize, usize)) -> Result<UserSelection> {
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
            // TODO deal with error conversion

            // erase this many lines of the regular print
            const ERASE: u16 = 4;
            // lines used by the question
            const QUESTION: u16 = 4;
            let extra_rows_to_flush = (state.n_items - (ERASE - QUESTION) as usize) as u16;
            stdout()
                .queue(cursor::Hide)
                .unwrap()
                .queue(cursor::MoveUp(ERASE)) // erase the 5 last lines of suggestion print
                .unwrap()
                .queue(terminal::Clear(terminal::ClearType::FromCursorDown))
                .unwrap()
                .queue(cursor::MoveDown(1)) // add a space between the question and the error
                .unwrap()
                .queue(PrintStyledContent(StyledContent::new(boring, question)))
                .unwrap()
                .queue(terminal::ScrollUp(extra_rows_to_flush))
                .unwrap()
                .queue(cursor::MoveToColumn(0))
                .unwrap()
                .queue(cursor::MoveDown(extra_rows_to_flush))
                .unwrap();
        }

        loop {
            let mut _guard = ScopedRaw::new();

            self.print_replacements_list(state)?;

            if state.is_custom_entry() {
                info!("Custom entry mode");

                stdout().queue(cursor::SavePosition).unwrap();
                stdout()
                    .queue(cursor::Show)
                    .unwrap()
                    .queue(cursor::MoveToPreviousLine(1))
                    .unwrap()
                    .queue(cursor::MoveToColumn(4 + state.cursor_offset))
                    .unwrap();
                let _ = stdout().flush();
            }

            let event = match crossterm::event::read()
                .map_err(|e| anyhow::anyhow!("Something unexpected happened on the CLI: {}", e))?
            {
                Event::Key(event) => event,
                Event::Resize(..) => {
                    drop(_guard);
                    continue;
                }
                sth => {
                    trace!("read() something other than a key: {:?}", sth);
                    break;
                }
            };

            if state.is_custom_entry() {
                drop(_guard);
                info!("Custom entry mode");
                _guard = ScopedRaw::new();

                let pick = self.enter_custom_replacement(state, event)?;

                stdout()
                    .queue(cursor::Hide)
                    .unwrap()
                    .queue(cursor::RestorePosition)
                    .unwrap();

                match pick {
                    UserSelection::Nop => continue,
                    other => return Ok(other),
                }
            }

            drop(_guard);
            // print normally again
            trace!("registered event: {:?}", &event);

            let KeyEvent { code, modifiers } = event;

            match code {
                KeyCode::Up => state.select_next(),
                KeyCode::Down => state.select_previous(),
                KeyCode::Enter | KeyCode::Char('y') => {
                    let kit: FirstAidKit = state.to_first_aid_kit();
                    // TODO handle interactive intput for those where there are no suggestions
                    return Ok(UserSelection::Replacement(kit));
                }
                KeyCode::Char('n') => return Ok(UserSelection::Skip),
                KeyCode::Char('j') => return Ok(UserSelection::Previous),
                KeyCode::Char('q') | KeyCode::Esc => return Ok(UserSelection::Abort),
                KeyCode::Char('c') if modifiers == KeyModifiers::CONTROL => {
                    return Ok(UserSelection::Abort)
                }
                KeyCode::Char('d') => return Ok(UserSelection::SkipFile),
                KeyCode::Char('e') => {
                    // jump to the user input entry
                    state.select_custom();
                }
                KeyCode::Char('?') => return Ok(UserSelection::Help),
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
    ) -> Result<(Self, UserSelection)> {
        let mut picked = UserPicked::default();

        trace!("Select the ones to actully use");

        for (origin, suggestions) in suggestions_per_path {
            let count = suggestions.len();
            trace!("Path is {} and has {}", origin, count);

            // TODO juck, uggly
            let mut suggestions_it = suggestions.clone().into_iter().enumerate();

            let mut direction = Direction::Forward;
            loop {
                let opt: Option<(usize, Suggestion)> = match direction {
                    Direction::Forward => suggestions_it.next(),
                    Direction::Backward => suggestions_it.next_back(), // FIXME TODO this is just plain wrong
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
                let (idx, suggestion) = opt.expect("Must be Some(_)");
                if suggestion.replacements.is_empty() {
                    trace!("Suggestion did not contain a replacement, skip");
                    continue;
                }
                println!("{}", suggestion);

                let mut state = State::from(&suggestion);

                let mut pick = picked.user_input(&mut state, (idx, count))?;
                while pick == UserSelection::Help {
                    println!("{}", HELP);
                    pick = picked.user_input(&mut state, (idx, count))?;
                }
                match pick {
                    UserSelection::Abort => return Ok((picked, UserSelection::Abort)),
                    UserSelection::SkipFile => break, // break the inner loop
                    UserSelection::Previous => {
                        unimplemented!("Requires a iterator which works bidrectionally")
                    }
                    UserSelection::Help => {
                        unreachable!("Help must not be reachable here, it is handled before")
                    }
                    UserSelection::Replacement(kit) => {
                        picked.add_first_aid_kit(&origin, kit);
                    }
                    _ => continue,
                };

                direction = Direction::Forward;
            }
        }
        Ok((picked, UserSelection::Nop))
    }
}
