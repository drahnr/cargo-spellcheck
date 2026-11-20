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

use std::io::{stdout, Write};

use super::learning::{DictionaryEntry, SessionResolutions};

mod learning;

#[cfg(test)]
mod tests;

const NAVIGATION_COMMANDS: &[(&str, &str)] = &[
    ("Up", "select above"),
    ("Down", "select below"),
    ("Home", "select bottom item"),
    ("End", "select top item"),
];

const RESOLUTION_COMMANDS: &[(&str, &str)] = &[
    ("Enter", "use selected action"),
    ("y", "use selected action"),
    ("n", "skip finding"),
    ("a", "open learning"),
    ("q", "apply choices and quit"),
    ("Esc", "apply choices and quit"),
    ("Ctrl-C", "discard choices and abort"),
    ("d", "skip remaining file"),
    ("j", "report unavailable navigation"),
    ("e", "edit replacement"),
];

const CUSTOM_COMMANDS: &[(&str, &str)] = &[
    ("Left", "move cursor left"),
    ("Right", "move cursor right"),
    ("Up", "select above"),
    ("Down", "select below"),
    ("Backspace", "delete previous character"),
    ("Enter", "use replacement"),
    ("Esc", "discard choices and abort"),
    ("Ctrl-C", "discard choices and abort"),
];

fn print_commands(
    output: &mut impl Write,
    commands: impl IntoIterator<Item = (&'static str, &'static str)>,
) -> Result<()> {
    write!(output, "\r\n")?;
    for (key, description) in commands {
        write!(output, "{key} - {description}\r\n")?;
    }
    write!(output, "? - show commands\r\n")?;
    output.flush()?;
    Ok(())
}

fn print_resolution_commands(
    output: &mut impl Write,
    custom: bool,
    can_accept: bool,
) -> Result<()> {
    let navigation = if custom { &[][..] } else { NAVIGATION_COMMANDS };
    let commands = if custom {
        CUSTOM_COMMANDS
    } else {
        RESOLUTION_COMMANDS
    };
    print_commands(
        output,
        navigation
            .iter()
            .chain(commands)
            .copied()
            .filter(|(key, _)| *key != "a" || can_accept),
    )
}

#[derive(Debug, PartialEq, Eq)]
enum SelectionAction {
    Redraw,
    Submit,
    Skip,
    Cancel,
    Abort,
}

fn select_key(selected: &mut usize, count: usize, key: KeyEvent) -> Option<SelectionAction> {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(SelectionAction::Abort);
    }
    match key.code {
        KeyCode::Up => *selected = (*selected + 1) % count,
        KeyCode::Down => *selected = (*selected + count - 1) % count,
        KeyCode::Home => *selected = 0,
        KeyCode::End => *selected = count - 1,
        KeyCode::Enter | KeyCode::Char('y') => return Some(SelectionAction::Submit),
        KeyCode::Char('n') => return Some(SelectionAction::Skip),
        KeyCode::Esc => return Some(SelectionAction::Cancel),
        _ => return None,
    }
    Some(SelectionAction::Redraw)
}

fn selection_style() -> ContentStyle {
    let mut style = ContentStyle::new();
    style.background_color = Some(Color::Black);
    style.foreground_color = Some(Color::Green);
    style.attributes = Attribute::Bold.into();
    style
}

fn print_selection_list<'a>(
    output: &mut impl Write,
    items: impl IntoIterator<Item = (ContentStyle, &'a str, Option<&'a str>)>,
    active_idx: usize,
    editable_idx: Option<usize>,
) -> Result<()> {
    let items = items.into_iter().collect::<Vec<_>>();
    let content_width = items
        .iter()
        .map(|(_, content, _)| console::measure_text_width(content))
        .max()
        .unwrap_or(0);
    let mut tick = ContentStyle::new();
    tick.foreground_color = Some(Color::Green);
    tick.attributes = Attribute::Bold.into();

    output.queue(cursor::SavePosition)?;
    for (idx, (style, content, description)) in items.into_iter().enumerate() {
        if idx > 0 {
            output.queue(cursor::MoveUp(1))?;
        }
        output.queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
        let style = if idx == active_idx {
            if editable_idx == Some(idx) {
                output.queue(cursor::Show)?;
            } else {
                output.queue(cursor::Hide)?;
            }
            output
                .queue(cursor::MoveToColumn(2))?
                .queue(PrintStyledContent(tick.apply('»')))?;
            selection_style()
        } else {
            style
        };
        output
            .queue(cursor::MoveToColumn(4))?
            .queue(PrintStyledContent(style.apply(content)))?;
        if let Some(description) = description {
            let padding = content_width - console::measure_text_width(content) + 1;
            write!(output, "{:padding$}< {description}", "")?;
        }
    }
    output.queue(cursor::RestorePosition)?.flush()?;
    Ok(())
}

struct FeedbackLine<W: Write> {
    output: W,
    visible: bool,
}

impl<W: Write> FeedbackLine<W> {
    fn new(output: W) -> Self {
        Self {
            output,
            visible: false,
        }
    }

    fn show(&mut self) -> Result<()> {
        self.visible = true;
        self.output
            .queue(cursor::MoveToColumn(0))?
            .queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
        write!(self.output, "next up...")?;
        self.output.flush()?;
        Ok(())
    }

    fn clear(&mut self) -> Result<()> {
        if self.visible {
            self.output
                .queue(cursor::MoveToColumn(0))?
                .queue(terminal::Clear(terminal::ClearType::CurrentLine))?
                .flush()?;
            self.visible = false;
        }
        Ok(())
    }
}

impl<W: Write> Drop for FeedbackLine<W> {
    fn drop(&mut self) {
        // Also remove the status on early returns and input errors.
        let _ = self.clear();
    }
}

/// Helper strict to assure we leave the terminals raw mode
pub struct ScopedRaw;

impl ScopedRaw {
    /// Enter raw terminal mode.
    ///
    /// Must be left before using `log::info!(..)` or any other printing macros
    /// or functions.
    fn new() -> Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self)
    }

    /// Helper to restore the previous terminal state.
    ///
    /// Also called on `drop`.
    pub fn restore_terminal() -> Result<()> {
        crossterm::terminal::disable_raw_mode()?;
        stdout()
            .queue(crossterm::cursor::Show)?
            .flush()
            .wrap_err_with(|| eyre!("Failed to restore terminal"))
    }
}

impl Drop for ScopedRaw {
    fn drop(&mut self) {
        let _ = Self::restore_terminal();
    }
}

/// The only document-level outcomes exposed to the session orchestrator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DocumentOutcome {
    Complete,
    SkipFile,
    Quit,
    Abort,
}

/// The user picked something. This is the pick representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum UserSelection {
    /// This `BandAid` is going to be applied.
    Replacement(BandAid),
    /// Accept the original spelling as project vocabulary.
    AcceptWord,
    /// Keep the source verbatim and select a dictionary entry before learning.
    LearnWord,
    /// A separately confirmed vocabulary entry, optionally accompanying a source edit.
    RecordEntry {
        entry: DictionaryEntry,
        forms: Vec<String>,
        replacement: Option<BandAid>,
    },
    /// Skip this suggestion and move on to the next suggestion.
    Skip,
    /// Jump to the previous suggestion.
    Previous,

    /// Skip the remaining fixes for the current file.
    SkipFile,
    /// continue as if whatever returned this was never called.
    Nop,
    /// Stop execution, forget all previous choices.
    Abort,
    /// Stop fixing chunks, move on to applying the ones chosen so far.
    Quit,
}

/// Statefulness for the selection process
#[derive(Debug)]
pub(super) struct State<'s, 't>
where
    't: 's,
{
    /// Which suggestion is operated upon.
    pub suggestion: &'s Suggestion<'t>,
    /// Eligibility supplied by the session's vocabulary policy.
    pub can_accept: bool,
    /// The content the user provided for the suggestion, if any.
    pub custom_replacement: String,
    pub cursor_offset: u16,
    /// Back ticked original content
    pub backticked_original: String,
    /// Which index to show as highlighted.
    pub pick_idx: usize,
    /// Total number of pickable slots.
    pub n_items: usize,
}

impl<'s, 't> State<'s, 't>
where
    't: 's,
{
    fn new(suggestion: &'s Suggestion<'t>, can_accept: bool) -> Self {
        Self {
            suggestion,
            can_accept,
            custom_replacement: String::new(),
            cursor_offset: 0,
            // TODO only suggest this if this doesn't have spaces and/or parses with `ap_syntax`
            // TODO and check the identifiers against everything we've seen in the codebase
            // TODO this has a few issues though, that partial runs might be unaware of all `Ident`s
            // TODO so there should probably be a strict mode for full runs, that checks the existence
            // TODO and the default, partial mode that is more forgiving
            backticked_original: format!(
                "`{}`",
                sub_chars(suggestion.chunk.as_str(), suggestion.range.clone())
            ),
            // Prefer a checker replacement, otherwise backticks; never default to learning.
            pick_idx: usize::from(can_accept)
                + 1
                + usize::from(!suggestion.replacements.is_empty()),
            n_items: suggestion.replacements.len() + 2 + usize::from(can_accept),
        }
    }

    fn custom_index(&self) -> usize {
        usize::from(self.can_accept)
    }

    pub fn select_custom(&mut self) {
        self.pick_idx = self.custom_index();
    }

    fn is_learning_entry(&self) -> bool {
        self.can_accept && self.pick_idx == 0
    }

    /// Checks if the currently selected line is the custom entry.
    pub fn is_custom_entry(&self) -> bool {
        self.pick_idx == self.custom_index()
    }

    pub fn is_ticked_entry(&self) -> bool {
        self.pick_idx == self.custom_index() + 1
    }

    // Display omits two replacement rows when none are available. Reserve them
    // before the prompt so its cursor movement cannot erase the source context.
    fn prompt_padding(&self) -> &'static str {
        if self.suggestion.replacements.is_empty() {
            "\n\n"
        } else {
            ""
        }
    }

    fn to_selection(&self) -> UserSelection {
        if self.is_learning_entry() {
            UserSelection::LearnWord
        } else {
            UserSelection::Replacement(self.to_bandaid())
        }
    }

    /// Convert the replacement to a `BandAid`
    pub fn to_bandaid(&self) -> BandAid {
        if self.is_ticked_entry() {
            BandAid::from((self.backticked_original.clone(), &self.suggestion.span))
        } else if self.is_custom_entry() {
            BandAid::from((self.custom_replacement.clone(), &self.suggestion.span))
        } else {
            let replacement = self
                .suggestion
                .replacements
                .get(self.pick_idx - self.custom_index() - 2)
                .expect("User Pick index is never out of bounds. qed");
            BandAid::from((replacement.to_owned(), &self.suggestion.span))
        }
    }
}

/// The selection of used suggestion replacements.
#[derive(Debug, Clone, Default)]
pub struct UserPicked {
    /// Associates the bandaids to a content origin, or path respectively.
    pub bandaids: indexmap::IndexMap<ContentOrigin, Vec<BandAid>>,
}

impl UserPicked {
    /// Count the number of suggestions across all files in total.
    pub fn total_count(&self) -> usize {
        self.bandaids
            .iter()
            .map(|(_origin, bandaids)| bandaids.len())
            .sum()
    }

    /// Are there any user picks?
    pub fn is_empty(&self) -> bool {
        !self
            .bandaids
            .iter()
            .any(|(_origin, bandaids)| !bandaids.is_empty())
    }

    /// Apply a single `BandAid`
    pub fn add_bandaid(&mut self, origin: &ContentOrigin, bandaid: BandAid) {
        self.bandaids
            .entry(origin.clone())
            .or_insert_with(|| Vec::with_capacity(10))
            .push(bandaid);
    }

    /// Apply multiple bandaids.
    pub fn add_bandaids<I>(&mut self, origin: &ContentOrigin, fixes: I)
    where
        I: IntoIterator<Item = BandAid>,
    {
        let iter = fixes.into_iter();
        self.bandaids
            .entry(origin.clone())
            .or_insert_with(|| Vec::with_capacity(iter.size_hint().0))
            .extend(iter);
    }

    /// Join two `UserPick`s.
    pub fn extend(&mut self, other: Self) {
        for (origin, bandaids) in other.bandaids {
            self.add_bandaids(&origin, bandaids);
        }
    }

    /// Provide a replacement that was not provided by the backend
    fn enter_custom_replacement(
        &self,
        state: &mut State,
        event: KeyEvent,
    ) -> Result<UserSelection> {
        let KeyEvent {
            code, modifiers, ..
        } = event;

        let length = state.custom_replacement.len() as u16;
        match code {
            KeyCode::Left => state.cursor_offset = state.cursor_offset.saturating_sub(1),
            KeyCode::Right => state.cursor_offset = (state.cursor_offset + 1).min(length),
            KeyCode::Up | KeyCode::Down => {
                state.cursor_offset = length;
                let _ = select_key(&mut state.pick_idx, state.n_items, event);
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
                let bandaid = state.to_bandaid();
                return Ok(UserSelection::Replacement(bandaid));
            }
            KeyCode::Esc => return Ok(UserSelection::Abort),
            KeyCode::Char('c') if modifiers == KeyModifiers::CONTROL => {
                return Ok(UserSelection::Abort);
            }
            KeyCode::Char('?') => {}
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
    // but now it's only a very simple list
    fn print_replacements_list(&self, state: &mut State) -> Result<()> {
        let mut others = ContentStyle::new();
        others.background_color = Some(Color::Black);
        others.foreground_color = Some(Color::Blue);

        let mut custom = others;
        custom.foreground_color = Some(Color::Yellow);
        let custom_content = if state.custom_replacement.is_empty() {
            "..."
        } else {
            state.custom_replacement.as_str()
        };
        let learning = state.can_accept.then(|| {
            (
                others,
                state
                    .suggestion
                    .checked_word
                    .as_deref()
                    .expect("eligible word"),
                Some("learn"),
            )
        });
        let items = learning
            .into_iter()
            .chain(std::iter::once((
                custom,
                custom_content,
                Some("custom input"),
            )))
            .chain(std::iter::once((
                others,
                state.backticked_original.as_str(),
                Some("wrap backticks"),
            )))
            .chain(
                state
                    .suggestion
                    .replacements
                    .iter()
                    .map(|s| (others, s.as_str(), None)),
            );
        print_selection_list(
            &mut stdout(),
            items,
            state.pick_idx,
            Some(state.custom_index()),
        )
    }

    /// Wait for user input and process it into a `UserSelection` enum.
    fn user_input(
        &self,
        state: &mut State,
        running_idx: usize,
        total: usize,
    ) -> Result<UserSelection> {
        let acceptance = if state.can_accept { ",a" } else { "" };
        let question = format!(
            "({nth}/{of_n}) Resolve this issue [y,n,q{acceptance},d,j,e,?]?",
            nth = running_idx + 1,
            of_n = total,
        );
        let mut boring = ContentStyle::new();
        boring.foreground_color = Some(Color::Blue);
        boring.attributes = Attribute::Bold.into();
        write!(stdout(), "{}", state.prompt_padding())?;
        {
            let _guard = ScopedRaw::new();

            // a new suggestion, so prepare for the number of items that are visible
            // and also overwrite the last lines of the regular print which would
            // already contain the suggestions
            // TODO deal with error conversion

            // erase this many lines of the regular print
            #[allow(clippy::items_after_statements)]
            const ERASE: u16 = 4;
            // lines used by the question
            const QUESTION: u16 = 3;
            let extra_rows_to_flush =
                (state.n_items.saturating_sub((ERASE - QUESTION) as usize)) as u16;
            stdout()
                .queue(cursor::Hide)?
                .queue(cursor::MoveUp(ERASE))? // erase the 5 last lines of suggestion print
                .queue(terminal::Clear(terminal::ClearType::FromCursorDown))?
                .queue(cursor::MoveDown(1))? // add a space between the question and the error
                .queue(PrintStyledContent(StyledContent::new(
                    boring,
                    question.as_str(),
                )))?
                .queue(terminal::ScrollUp(extra_rows_to_flush))?
                .queue(cursor::MoveToColumn(0))?
                .queue(cursor::MoveDown(extra_rows_to_flush))?;
        }

        loop {
            let mut _guard = ScopedRaw::new();

            self.print_replacements_list(state)?;

            if state.is_custom_entry() {
                let mut output = stdout();
                output.queue(cursor::SavePosition)?.queue(cursor::Show)?;
                if state.custom_index() > 0 {
                    output.queue(cursor::MoveUp(state.custom_index() as u16))?;
                }
                output.queue(cursor::MoveToColumn(4 + state.cursor_offset))?;
                stdout().flush()?;
            }

            let event = match crossterm::event::read()
                .wrap_err_with(|| eyre!("Something unexpected happened on the CLI"))?
            {
                Event::Key(event) => event,
                Event::Resize(..) => {
                    drop(_guard);
                    continue;
                }
                sth => {
                    log::trace!("read() something other than a key: {sth:?}");
                    break;
                }
            };

            if event.code == KeyCode::Char('?') {
                let mut output = stdout();
                if state.is_custom_entry() {
                    output.queue(cursor::RestorePosition)?;
                }
                print_resolution_commands(&mut output, state.is_custom_entry(), state.can_accept)?;
                output.queue(PrintStyledContent(boring.apply(question.as_str())))?;
                // Help can scroll; give the unchanged picker a fresh anchor.
                for _ in 0..state.n_items + 1 {
                    write!(output, "\r\n")?;
                }
                output.flush()?;
                continue;
            }

            if state.is_custom_entry() {
                drop(_guard);
                log::info!("Custom entry mode");
                _guard = ScopedRaw::new();

                let pick = self.enter_custom_replacement(state, event)?;

                stdout()
                    .queue(cursor::Hide)?
                    .queue(cursor::RestorePosition)?;

                match pick {
                    UserSelection::Nop => continue,
                    other => return Ok(other),
                }
            }

            drop(_guard);
            // print normally again
            log::trace!("registered event: {event:?}");

            match select_key(&mut state.pick_idx, state.n_items, event) {
                Some(SelectionAction::Redraw) => continue,
                Some(SelectionAction::Submit) => {
                    return Ok(state.to_selection());
                }
                Some(SelectionAction::Skip) => return Ok(UserSelection::Skip),
                Some(SelectionAction::Cancel) => return Ok(UserSelection::Quit),
                Some(SelectionAction::Abort) => return Ok(UserSelection::Abort),
                None => {}
            }

            match event.code {
                KeyCode::Char('a') if state.can_accept => {
                    return Ok(UserSelection::AcceptWord);
                }
                KeyCode::Char('j') => return Ok(UserSelection::Previous),
                KeyCode::Char('q') => return Ok(UserSelection::Quit),
                KeyCode::Char('d') => return Ok(UserSelection::SkipFile),
                KeyCode::Char('e') => {
                    // jump to the user input entry
                    state.select_custom();
                }

                x => {
                    log::trace!("Unexpected input {x:?}");
                }
            }
        }
        unreachable!("Unexpected return when dealing with user input")
    }

    pub(super) fn select_interactive<'s>(
        origin: ContentOrigin,
        suggestions: Vec<Suggestion<'s>>,
        resolutions: &mut SessionResolutions,
        checkers: &Checkers,
        output_paths: &OutputPaths,
    ) -> Result<(Self, DocumentOutcome)> {
        Self::select_with_followup(
            origin,
            suggestions,
            resolutions,
            output_paths,
            |picked, state, idx, count, resolutions| {
                let selection = picked.user_input(state, idx, count)?;
                let (word, accept_as_is) = match &selection {
                    UserSelection::AcceptWord | UserSelection::LearnWord => {
                        match state.suggestion.checked_word.as_deref() {
                            Some(word) => (word, true),
                            None => return Ok(selection),
                        }
                    }
                    UserSelection::Replacement(replacement)
                        if state.can_accept
                            && !state.is_ticked_entry()
                            && resolutions
                                .validate_learning_word(&replacement.content)
                                .is_ok()
                            && !resolutions.knows_word(&replacement.content)
                            && !checkers.contains_word(&replacement.content) =>
                    {
                        (replacement.content.as_str(), false)
                    }
                    _ => return Ok(selection),
                };
                let outcome = if matches!(selection, UserSelection::LearnWord) {
                    learning::choose_word(word, resolutions)?
                } else {
                    learning::prompt(word, accept_as_is, resolutions)?
                };
                match outcome {
                    learning::LearningOutcome::Record(entry, forms) => {
                        Ok(UserSelection::RecordEntry {
                            entry,
                            forms,
                            replacement: match selection {
                                UserSelection::Replacement(replacement) => Some(replacement),
                                _ => None,
                            },
                        })
                    }
                    learning::LearningOutcome::Skip => Ok(selection),
                    learning::LearningOutcome::Cancel => Ok(UserSelection::Nop),
                    learning::LearningOutcome::Abort => Ok(UserSelection::Abort),
                }
            },
        )
    }

    #[cfg(test)]
    pub(super) fn select_with<'s>(
        origin: ContentOrigin,
        suggestions: Vec<Suggestion<'s>>,
        resolutions: &mut SessionResolutions,
        input: impl FnMut(&Self, &mut State<'_, 's>, usize, usize) -> Result<UserSelection>,
    ) -> Result<(Self, DocumentOutcome)> {
        let output_paths = OutputPaths::new(&[], false)?;
        Self::select_with_paths(origin, suggestions, resolutions, &output_paths, input)
    }

    #[cfg(test)]
    fn select_with_paths<'s>(
        origin: ContentOrigin,
        suggestions: Vec<Suggestion<'s>>,
        resolutions: &mut SessionResolutions,
        output_paths: &OutputPaths,
        mut input: impl FnMut(&Self, &mut State<'_, 's>, usize, usize) -> Result<UserSelection>,
    ) -> Result<(Self, DocumentOutcome)> {
        Self::select_with_followup(
            origin,
            suggestions,
            resolutions,
            output_paths,
            |picked, state, idx, count, _| input(picked, state, idx, count),
        )
    }

    fn select_with_followup<'s>(
        origin: ContentOrigin,
        suggestions: Vec<Suggestion<'s>>,
        resolutions: &mut SessionResolutions,
        output_paths: &OutputPaths,
        mut input: impl FnMut(
            &Self,
            &mut State<'_, 's>,
            usize,
            usize,
            &mut SessionResolutions,
        ) -> Result<UserSelection>,
    ) -> Result<(Self, DocumentOutcome)> {
        let count = suggestions.len();
        let mut picked = UserPicked::default();
        let mut feedback = FeedbackLine::new(stdout());

        for (idx, suggestion) in suggestions.iter().enumerate() {
            // Suggestions are generated before interaction; consult session state
            // here so acceptance also suppresses queued duplicates in this chunk.
            if resolutions.accepts(suggestion) {
                continue;
            }
            feedback.clear()?;
            let path = output_paths.path(suggestion.origin.as_path());
            println!("{}", suggestion.display_with_path(&path));

            let mut state = State::new(suggestion, resolutions.can_accept(suggestion));

            'inner: loop {
                match input(&picked, &mut state, idx, count, resolutions)? {
                    UserSelection::Abort => return Ok((picked, DocumentOutcome::Abort)),
                    UserSelection::Quit => return Ok((picked, DocumentOutcome::Quit)),
                    UserSelection::SkipFile => return Ok((picked, DocumentOutcome::SkipFile)),
                    UserSelection::Previous => {
                        log::warn!("Requires an iterator which works bi-directionally");
                        continue 'inner;
                    }

                    UserSelection::AcceptWord | UserSelection::LearnWord => {
                        if let Err(error) = resolutions.accept(suggestion) {
                            log::warn!("{}", error);
                            continue 'inner;
                        }
                    }
                    UserSelection::RecordEntry {
                        entry,
                        forms,
                        replacement,
                    } => {
                        if let Err(error) = resolutions.record_entry(entry, &forms) {
                            log::warn!("{error}");
                            println!("{}", suggestion.display_with_path(&path));
                            continue 'inner;
                        }
                        if let Some(replacement) = replacement {
                            picked.add_bandaid(&origin, replacement);
                        }
                    }
                    UserSelection::Replacement(bandaid) => {
                        picked.add_bandaid(&origin, bandaid);
                    }
                    UserSelection::Nop => {
                        println!("{}", suggestion.display_with_path(&path));
                        continue 'inner;
                    }
                    UserSelection::Skip => {}
                };
                feedback.show()?;
                break 'inner;
            }
        }
        feedback.clear()?;
        Ok((picked, DocumentOutcome::Complete))
    }
}
