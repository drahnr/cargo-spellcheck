//! Read-only learning prompts; the caller owns recording and persistence.

use super::{
    print_commands, print_selection_list, select_key, selection_style, ScopedRaw, SelectionAction,
    NAVIGATION_COMMANDS,
};
use crate::action::learning::{AffixSuggestion, DictionaryEntry, SessionResolutions};
use crate::errors::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    style::{Color, ContentStyle, PrintStyledContent},
    terminal, QueueableCommand,
};
use std::io::{stdout, Write};

#[derive(Debug)]
pub(super) enum LearningOutcome {
    Skip,
    Record(DictionaryEntry, Vec<String>),
    Cancel,
    Abort,
}

pub(super) fn prompt(
    word: &str,
    accept_as_is: bool,
    session: &SessionResolutions,
) -> Result<LearningOutcome> {
    // Keep raw mode active across every stage, including queued input.
    let _raw = ScopedRaw::new()?;
    print_learning_mode(session)?;
    print_learning_prompt(&mut stdout(), word, accept_as_is)?;
    loop {
        let key = read_key()?;
        if key.code == KeyCode::Char('?') {
            print_learning_commands(&mut stdout(), accept_as_is)?;
            print_learning_options(&mut stdout(), word, accept_as_is)?;
            continue;
        }
        write!(stdout(), "\r\n")?;
        if is_control(key, 'c') {
            return Ok(LearningOutcome::Abort);
        }
        let outcome = match key.code {
            KeyCode::Char('p') | KeyCode::Enter => choose_and_confirm(word, session)?,
            KeyCode::Esc | KeyCode::Char('n') => None,
            KeyCode::Char('e') => edit_and_choose(word, session)?,
            _ => {
                print_learning_prompt(&mut stdout(), word, accept_as_is)?;
                continue;
            }
        };
        return Ok(outcome.unwrap_or(if accept_as_is {
            LearningOutcome::Cancel
        } else {
            LearningOutcome::Skip
        }));
    }
}

pub(super) fn choose_word(word: &str, session: &SessionResolutions) -> Result<LearningOutcome> {
    session.validate_learning_word(word)?;
    let _raw = ScopedRaw::new()?;
    print_learning_mode(session)?;
    Ok(choose_and_confirm(word, session)?.unwrap_or(LearningOutcome::Cancel))
}

fn print_learning_mode(session: &SessionResolutions) -> Result<()> {
    if session.is_persistent() {
        write!(stdout(), "\r\nPersistent learning: recorded entries will be saved when this session is applied.\r\n")?;
    } else {
        write!(stdout(), "\r\nSession-only learning: entries are not saved; configure learning_dictionary to persist them.\r\n")?;
    }
    Ok(())
}

fn print_entry_preview(
    output: &mut impl Write,
    entry: &DictionaryEntry,
    forms: &[String],
) -> Result<()> {
    write!(output, "New entry: ")?;
    output.queue(PrintStyledContent(selection_style().apply(entry.to_line())))?;
    write!(output, "\r\nAffix expansion: ")?;
    if entry.flags().is_none() {
        write!(output, "{} (plain word; no affixes)\r\n", entry.word())?;
    } else if forms.is_empty() {
        write!(
            output,
            "unavailable (backend supplied no expanded forms)\r\n"
        )?;
    } else {
        write!(output, "{}\r\n", forms.join(", "))?;
    }
    Ok(())
}

fn print_learning_prompt(output: &mut impl Write, word: &str, accept_as_is: bool) -> Result<()> {
    print_entry_preview(output, &DictionaryEntry::plain(word)?, &[])?;
    print_learning_options(output, word, accept_as_is)
}

fn print_learning_options(output: &mut impl Write, word: &str, accept_as_is: bool) -> Result<()> {
    let replacement = if accept_as_is { "" } else { " replacement" };
    let cancel = if accept_as_is {
        "cancel"
    } else {
        "skip learning"
    };
    write!(output, "Learn{replacement} \"")?;
    output.queue(PrintStyledContent(selection_style().apply(word)))?;
    write!(
        output,
        "\" [Enter/p=affixes, e=edit word, n/Esc={cancel}, Ctrl-C=abort, ?=help]: "
    )?;
    Ok(())
}

fn print_learning_commands(output: &mut impl Write, accept_as_is: bool) -> Result<()> {
    let cancel = if accept_as_is {
        "cancel learning"
    } else {
        "skip learning"
    };
    print_commands(
        output,
        [
            ("Enter", "choose affixes"),
            ("p", "choose affixes"),
            ("e", "edit word"),
            ("n", cancel),
            ("Esc", cancel),
            ("Ctrl-C", "discard choices and abort"),
        ],
    )
}

fn edit_and_choose(word: &str, session: &SessionResolutions) -> Result<Option<LearningOutcome>> {
    let mut edited = word.to_owned();
    loop {
        let (action, text) = read_line(EditorStage::Word, &edited)?;
        match action {
            Input::Abort => return Ok(Some(LearningOutcome::Abort)),
            Input::Cancel => return Ok(None),
            Input::Submit => edited = text,
        }
        match session.validate_learning_word(&edited) {
            Ok(()) => break,
            Err(error) => write!(stdout(), "Invalid word: {error}\r\n")?,
        }
    }

    choose_and_confirm(&edited, session)
}

fn choose_and_confirm(word: &str, session: &SessionResolutions) -> Result<Option<LearningOutcome>> {
    let mut suffixes = vec![session.preview_entry(word, None)?];
    match session.entry_suggestions(word) {
        Ok(suggestions) => suffixes.extend(suggestions),
        Err(error) => write!(
            stdout(),
            "{error}; plain-word learning is still available.\r\n"
        )?,
    }
    'suffix: loop {
        let (action, selected) = choose_entry(&suffixes, EntryStage::Suffix)?;
        match action {
            Input::Abort => return Ok(Some(LearningOutcome::Abort)),
            Input::Cancel => return Ok(None),
            Input::Submit => {}
        }
        let flags = suffixes[selected].entry.flags();
        let stems = match flags {
            Some(flags) => session.stem_suggestions(word, flags)?,
            None => vec![suffixes[selected].clone()],
        };
        'stem: loop {
            let selected = if flags.is_some() {
                let (action, selected) = choose_entry(&stems, EntryStage::Stem)?;
                match action {
                    Input::Abort => return Ok(Some(LearningOutcome::Abort)),
                    Input::Cancel => continue 'suffix,
                    Input::Submit => selected,
                }
            } else {
                0
            };
            let mut stem = stems[selected].entry.word().to_owned();
            'edit: loop {
                let (action, edited) = read_line(EditorStage::Stem, &stem)?;
                match action {
                    Input::Abort => return Ok(Some(LearningOutcome::Abort)),
                    Input::Cancel if flags.is_some() => continue 'stem,
                    Input::Cancel => continue 'suffix,
                    Input::Submit => stem = edited,
                }
                let preview = match session.preview_entry(&stem, flags) {
                    Ok(preview) => preview,
                    Err(error) => {
                        write!(stdout(), "Invalid stem: {error}\r\n")?;
                        continue 'edit;
                    }
                };
                print_entry_preview(&mut stdout(), &preview.entry, &preview.forms)?;
                write!(stdout(), "{CONFIRMATION_PROMPT}")?;
                loop {
                    let key = read_key()?;
                    if key.code == KeyCode::Char('?') {
                        print_confirmation_commands(&mut stdout())?;
                        write!(stdout(), "{CONFIRMATION_PROMPT}")?;
                        continue;
                    }
                    if is_control(key, 'c') {
                        write!(stdout(), "\r\n")?;
                        return Ok(Some(LearningOutcome::Abort));
                    }
                    match key.code {
                        KeyCode::Char('y')
                            if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                        {
                            write!(stdout(), "\r\n")?;
                            return Ok(Some(LearningOutcome::Record(preview.entry, preview.forms)));
                        }
                        KeyCode::Char('e') => {
                            write!(stdout(), "\r\n")?;
                            continue 'edit;
                        }
                        KeyCode::Enter | KeyCode::Esc | KeyCode::Char('n') => {
                            write!(stdout(), "\r\n")?;
                            return Ok(None);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

const CONFIRMATION_PROMPT: &str = "Confirm learning [y=yes, Enter/n/Esc=no, e=edit stem, ?=help]: ";

fn print_confirmation_commands(output: &mut impl Write) -> Result<()> {
    print_commands(
        output,
        [
            ("y", "record entry"),
            ("Enter", "decline learning"),
            ("n", "decline learning"),
            ("Esc", "decline learning"),
            ("e", "edit stem"),
            ("Ctrl-C", "discard choices and abort"),
        ],
    )
}

#[derive(Clone, Copy)]
enum EntryStage {
    Suffix,
    Stem,
}

impl EntryStage {
    fn prompt(self) -> &'static str {
        match self {
            Self::Suffix => "Suffix [Up/Down=select, Enter=stems, n/Esc=back, ?=help]: ",
            Self::Stem => "Stem [Up/Down=select, Enter=edit, n/Esc=back, ?=help]: ",
        }
    }
}

fn choose_entry(choices: &[AffixSuggestion], stage: EntryStage) -> Result<(Input, usize)> {
    let mut selected = 0;
    let mut output = stdout();
    let (width, height) = terminal::size().unwrap_or((80, 24));
    let visible = choices
        .len()
        .min(usize::from(height.saturating_sub(3).max(1)));
    mount_picker(&mut output, stage, visible)?;
    loop {
        print_entries(&mut output, choices, selected, width, height, stage)?;
        print_picker_expansion(&mut output, &choices[selected], width)?;
        let key = read_key()?;
        if key.code == KeyCode::Char('?') {
            print_picker_commands(&mut output, stage, visible)?;
            continue;
        }
        if let Some(action) = select_entry_key(&mut selected, choices.len(), key) {
            write!(output, "\r\n\r\n")?;
            return Ok((action, selected));
        }
    }
}

fn mount_picker(output: &mut impl Write, stage: EntryStage, visible: usize) -> Result<()> {
    write!(output, "{}", stage.prompt())?;
    // Reserve a question, a gap, the list, and a separate expansion context row.
    for _ in 0..visible + 2 {
        write!(output, "\r\n")?;
    }
    output.queue(cursor::MoveUp(1))?;
    Ok(())
}

fn print_picker_commands(output: &mut impl Write, stage: EntryStage, visible: usize) -> Result<()> {
    let (submit, back) = match stage {
        EntryStage::Suffix => ("choose stems", "cancel learning"),
        EntryStage::Stem => ("edit stem", "return to suffixes"),
    };
    // The renderer leaves the cursor at the list base, above the expansion row.
    // Help may scroll, so mount a fresh picker rather than restoring old coordinates.
    write!(output, "\r\n\r\n")?;
    print_commands(
        output,
        NAVIGATION_COMMANDS.iter().copied().chain([
            ("Enter", submit),
            ("y", submit),
            ("n", back),
            ("Esc", back),
            ("Ctrl-C", "discard choices and abort"),
        ]),
    )?;
    mount_picker(output, stage, visible)
}

fn print_entries(
    output: &mut impl Write,
    choices: &[AffixSuggestion],
    selected: usize,
    width: u16,
    height: u16,
    stage: EntryStage,
) -> Result<u16> {
    // Leave room for the picker's four-column indent and avoid wrapping previews.
    let width = usize::from(width.saturating_sub(5));
    let visible = choices
        .len()
        .min(usize::from(height.saturating_sub(3).max(1)));
    let start = selected.saturating_sub(visible - 1);
    let rows = choices
        .iter()
        .skip(start)
        .take(visible)
        .map(|suggestion| {
            let entry = &suggestion.entry;
            let (content, description) = match (stage, entry.flags()) {
                (_, None) => (entry.word().to_owned(), "plain"),
                (EntryStage::Suffix, Some(flags)) => (format!("/{flags}"), "suffix"),
                (EntryStage::Stem, Some(_)) => (entry.to_line(), "stem"),
            };
            let available = width.saturating_sub(description.len() + 3);
            (
                console::truncate_str(&content, available, "…").into_owned(),
                description,
            )
        })
        .collect::<Vec<_>>();
    let mut style = ContentStyle::new();
    style.background_color = Some(Color::Black);
    style.foreground_color = Some(Color::Blue);
    print_selection_list(
        output,
        rows.iter()
            .map(|(content, description)| (style, content.as_str(), Some(*description))),
        selected - start,
        None,
    )?;
    Ok(visible as u16)
}

fn print_picker_expansion(
    output: &mut impl Write,
    suggestion: &AffixSuggestion,
    width: u16,
) -> Result<()> {
    let expansion = if suggestion.entry.flags().is_none() {
        format!("{} (plain word; no affixes)", suggestion.entry.word())
    } else {
        suggestion.forms.join(", ")
    };
    let context = format!("Affix expansion: {expansion}");
    output
        .queue(cursor::SavePosition)?
        .queue(cursor::MoveDown(1))?
        .queue(cursor::MoveToColumn(0))?
        .queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
    write!(
        output,
        "{}",
        console::truncate_str(&context, usize::from(width.saturating_sub(1)), "…")
    )?;
    output.queue(cursor::RestorePosition)?.flush()?;
    Ok(())
}

fn select_entry_key(selected: &mut usize, count: usize, key: KeyEvent) -> Option<Input> {
    match select_key(selected, count, key) {
        Some(SelectionAction::Submit) => Some(Input::Submit),
        Some(SelectionAction::Skip | SelectionAction::Cancel) => Some(Input::Cancel),
        Some(SelectionAction::Abort) => Some(Input::Abort),
        Some(SelectionAction::Redraw) | None => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Input {
    Submit,
    Cancel,
    Abort,
}

fn is_control(key: KeyEvent, character: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(character)
}

fn read_key() -> Result<KeyEvent> {
    stdout().flush()?;
    loop {
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Release {
                return Ok(key);
            }
        }
    }
}

fn edit_key(text: &mut String, key: KeyEvent) -> Option<Input> {
    if is_control(key, 'c') {
        return Some(Input::Abort);
    }
    if is_control(key, 'u') {
        text.clear();
        return None;
    }
    match key.code {
        KeyCode::Enter => return Some(Input::Submit),
        KeyCode::Esc => return Some(Input::Cancel),
        KeyCode::Char('?') => {}
        KeyCode::Backspace => {
            text.pop();
        }
        KeyCode::Char(ch)
            if !ch.is_control()
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            text.push(ch)
        }
        _ => {}
    }
    None
}

#[derive(Clone, Copy)]
enum EditorStage {
    Word,
    Stem,
}

impl EditorStage {
    fn prompt(self) -> &'static str {
        match self {
            Self::Word => "Word (plain word only; Enter=preview, Esc=back, ?=help): ",
            Self::Stem => "Stem (plain word only; Enter=preview, Esc=back, ?=help): ",
        }
    }

    fn print_commands(self, output: &mut impl Write) -> Result<()> {
        let (submit, back) = match self {
            Self::Word => ("choose affixes", "cancel learning"),
            Self::Stem => ("preview expansion", "return to picker"),
        };
        print_commands(
            output,
            [
                ("Backspace", "delete last character"),
                ("Ctrl-U", "clear input"),
                ("Enter", submit),
                ("Esc", back),
                ("Ctrl-C", "discard choices and abort"),
            ],
        )
    }
}

fn read_line(stage: EditorStage, initial: &str) -> Result<(Input, String)> {
    let mut text = initial.to_owned();
    loop {
        write!(stdout(), "\r\x1b[2K{}{text}", stage.prompt())?;
        let key = read_key()?;
        if key.code == KeyCode::Char('?') {
            stage.print_commands(&mut stdout())?;
            continue;
        }
        if let Some(action) = edit_key(&mut text, key) {
            write!(stdout(), "\r\n")?;
            return Ok((action, text));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learning_prompt_highlights_the_word_and_offers_enter_to_learn() {
        for accept_as_is in [false, true] {
            let mut output = Vec::new();
            print_learning_prompt(&mut output, "MSRV", accept_as_is).unwrap();
            let raw = String::from_utf8(output).unwrap();
            assert!(raw.contains(&selection_style().apply("MSRV").to_string()));
            assert!(console::strip_ansi_codes(&raw).contains("\"MSRV\" [Enter/p=affixes"));
            let cancel = if accept_as_is {
                "cancel"
            } else {
                "skip learning"
            };
            assert!(raw.ends_with(&format!(
                " [Enter/p=affixes, e=edit word, n/Esc={cancel}, Ctrl-C=abort, ?=help]: "
            )));
            assert!(!raw.contains("Enter/n/Esc"));
        }
    }

    #[test]
    fn learning_help_lists_each_key_and_the_current_cancel_effect() {
        for accept_as_is in [false, true] {
            let mut output = Vec::new();
            print_learning_commands(&mut output, accept_as_is).unwrap();
            let cancel = if accept_as_is {
                "cancel learning"
            } else {
                "skip learning"
            };
            assert_eq!(
                String::from_utf8(output).unwrap(),
                format!(
                    "\r\nEnter - choose affixes\r\np - choose affixes\r\ne - edit word\r\nn - {cancel}\r\nEsc - {cancel}\r\nCtrl-C - discard choices and abort\r\n? - show commands\r\n"
                )
            );
        }
    }

    #[test]
    fn confirmation_help_distinguishes_record_decline_and_edit() {
        let mut output = Vec::new();
        print_confirmation_commands(&mut output).unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\r\ny - record entry\r\nEnter - decline learning\r\nn - decline learning\r\nEsc - decline learning\r\ne - edit stem\r\nCtrl-C - discard choices and abort\r\n? - show commands\r\n"
        );
        assert_eq!(
            CONFIRMATION_PROMPT,
            "Confirm learning [y=yes, Enter/n/Esc=no, e=edit stem, ?=help]: "
        );
    }

    #[test]
    fn editor_help_preserves_input_and_describes_the_current_preview() {
        for (stage, name, submit, back) in [
            (
                EditorStage::Word,
                "Word",
                "choose affixes",
                "cancel learning",
            ),
            (
                EditorStage::Stem,
                "Stem",
                "preview expansion",
                "return to picker",
            ),
        ] {
            let mut text = "naïve🦀".to_owned();
            let mut output = Vec::new();
            assert_eq!(edit_key(&mut text, KeyCode::Char('?').into()), None);
            stage.print_commands(&mut output).unwrap();
            assert_eq!(text, "naïve🦀");
            assert_eq!(
                String::from_utf8(output).unwrap(),
                format!(
                    "\r\nBackspace - delete last character\r\nCtrl-U - clear input\r\nEnter - {submit}\r\nEsc - {back}\r\nCtrl-C - discard choices and abort\r\n? - show commands\r\n"
                )
            );
            assert_eq!(
                stage.prompt(),
                format!("{name} (plain word only; Enter=preview, Esc=back, ?=help): ")
            );
        }
    }

    #[test]
    fn picker_help_remounts_without_restoring_stale_coordinates_or_changing_selection() {
        let choices = ["first", "second", "third"]
            .into_iter()
            .map(|word| AffixSuggestion {
                entry: DictionaryEntry::with_test_flags(word, "S").unwrap(),
                forms: vec![format!("{word}s")],
            })
            .collect::<Vec<_>>();
        for (stage, name, submit, back) in [
            (
                EntryStage::Suffix,
                "Suffix",
                "choose stems",
                "cancel learning",
            ),
            (EntryStage::Stem, "Stem", "edit stem", "return to suffixes"),
        ] {
            let mut selected = 2;
            let mut output = Vec::new();
            print_entries(&mut output, &choices, selected, 80, 5, stage).unwrap();
            print_picker_expansion(&mut output, &choices[selected], 80).unwrap();
            assert_eq!(
                select_entry_key(&mut selected, choices.len(), KeyCode::Char('?').into()),
                None
            );
            let help_start = output.len();
            print_picker_commands(&mut output, stage, 2).unwrap();
            let help = std::str::from_utf8(&output[help_start..]).unwrap();
            assert!(help.starts_with("\r\n\r\n\r\n"));
            for (key, effect) in NAVIGATION_COMMANDS.iter().copied().chain([
                ("Enter", submit),
                ("y", submit),
                ("n", back),
                ("Esc", back),
                ("Ctrl-C", "discard choices and abort"),
                ("?", "show commands"),
            ]) {
                assert!(help.contains(&format!("{key} - {effect}\r\n")));
            }
            assert!(help.contains(&format!("? - show commands\r\n{name} [")));
            assert!(stage.prompt().ends_with(", ?=help]: "));
            assert!(help.ends_with(&format!("{}\r\n\r\n\r\n\r\n\x1b[1A", stage.prompt())));
            assert!(!help.contains("\x1b8"));
            assert_eq!(selected, 2);
            assert_eq!(choices[selected].entry.to_line(), "third/S");
            assert_eq!(choices[selected].forms, ["thirds"]);

            print_entries(&mut output, &choices, selected, 80, 5, stage).unwrap();
            print_picker_expansion(&mut output, &choices[selected], 80).unwrap();
            let raw = String::from_utf8(output).unwrap();
            // Each redraw restores once for the list and once for the expansion.
            assert_eq!(raw.matches("\x1b8").count(), 4);
            assert_eq!(console::strip_ansi_codes(&raw).matches('»').count(), 2);
        }
    }

    #[test]
    fn question_mark_is_reserved_even_with_editor_modifiers() {
        let mut text = "original".to_owned();
        for modifiers in [
            KeyModifiers::NONE,
            KeyModifiers::SHIFT,
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
        ] {
            assert_eq!(
                edit_key(&mut text, KeyEvent::new(KeyCode::Char('?'), modifiers)),
                None
            );
            assert_eq!(text, "original");
        }
    }

    #[test]
    fn plain_entry_preview_explicitly_has_no_affixes() {
        let mut output = Vec::new();
        print_entry_preview(&mut output, &DictionaryEntry::plain("MSRV").unwrap(), &[]).unwrap();
        let raw = String::from_utf8(output).unwrap();
        assert!(raw.contains(&selection_style().apply("MSRV").to_string()));
        assert_eq!(
            console::strip_ansi_codes(&raw),
            "New entry: MSRV\r\nAffix expansion: MSRV (plain word; no affixes)\r\n"
        );
    }

    #[test]
    fn flagged_entry_preview_shows_exact_entry_and_all_supplied_forms() {
        let entry = DictionaryEntry::with_test_flags("widget", "S").unwrap();
        let forms = vec!["widget".to_owned(), "widgets".to_owned()];
        let mut output = Vec::new();
        print_entry_preview(&mut output, &entry, &forms).unwrap();
        let raw = String::from_utf8(output).unwrap();
        assert!(raw.contains(&selection_style().apply("widget/S").to_string()));
        assert_eq!(
            console::strip_ansi_codes(&raw),
            "New entry: widget/S\r\nAffix expansion: widget, widgets\r\n"
        );
    }

    #[test]
    fn missing_expansion_is_not_presented_as_a_complete_preview() {
        let entry = DictionaryEntry::with_test_flags("widget", "S").unwrap();
        let mut output = Vec::new();
        print_entry_preview(&mut output, &entry, &[]).unwrap();
        let raw = String::from_utf8(output).unwrap();
        assert_eq!(
            console::strip_ansi_codes(&raw),
            "New entry: widget/S\r\nAffix expansion: unavailable (backend supplied no expanded forms)\r\n"
        );
    }

    #[test]
    fn entry_navigation_wraps_and_supports_home_and_end() {
        let mut selected = 0;
        for (key, expected) in [
            (KeyCode::Up, 1),
            (KeyCode::Down, 0),
            (KeyCode::Down, 2),
            (KeyCode::End, 2),
            (KeyCode::Home, 0),
        ] {
            assert_eq!(select_entry_key(&mut selected, 3, key.into()), None);
            assert_eq!(selected, expected);
        }
        assert_eq!(select_entry_key(&mut selected, 1, KeyCode::Up.into()), None);
        assert_eq!(selected, 0);
        assert_eq!(
            select_entry_key(&mut selected, 1, KeyCode::Down.into()),
            None
        );
        assert_eq!(selected, 0);
    }

    #[test]
    fn entry_selection_preserves_confirmation_cancellation_and_abort() {
        let mut selected = 1;
        for (key, expected) in [
            (KeyCode::Enter.into(), Input::Submit),
            (KeyCode::Char('y').into(), Input::Submit),
            (KeyCode::Esc.into(), Input::Cancel),
            (KeyCode::Char('n').into(), Input::Cancel),
            (
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                Input::Abort,
            ),
        ] {
            assert_eq!(select_entry_key(&mut selected, 3, key), Some(expected));
            assert_eq!(selected, 1);
        }
    }

    #[test]
    fn entry_preview_keeps_the_highlight_visible_without_wrapping() {
        let choices = ["first", "second", "naïve🦀longword"]
            .into_iter()
            .map(|word| AffixSuggestion {
                entry: DictionaryEntry::plain(word).unwrap(),
                forms: vec![word.to_owned()],
            })
            .collect::<Vec<_>>();
        let mut output = Vec::new();
        assert_eq!(
            print_entries(&mut output, &choices, 2, 24, 5, EntryStage::Stem).unwrap(),
            2
        );
        let raw = String::from_utf8(output).unwrap();
        let text = console::strip_ansi_codes(&raw);
        assert!(!text.contains("first"));
        assert!(text.contains("second"));
        let selected = console::truncate_str("naïve🦀longword", 11, "…");
        assert!(raw.contains(&format!("{} < plain", selection_style().apply(selected))));
        assert_eq!(text.matches('»').count(), 1);
        assert!(!text.contains("2:"));
        assert!(raw.starts_with("\x1b7\x1b[2K"));
        assert_eq!(raw.matches("\x1b[1A\x1b[2K").count(), 1);
        assert!(raw.ends_with("\x1b8"));
    }

    #[test]
    fn editing_preserves_unicode_and_clears_prefilled_words() {
        let mut word = "naïve🦀".to_owned();
        assert_eq!(edit_key(&mut word, KeyCode::Backspace.into()), None);
        assert_eq!(word, "naïve");
        edit_key(&mut word, KeyCode::Char('é').into());
        assert_eq!(word, "naïveé");
        edit_key(
            &mut word,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        assert!(word.is_empty());
        edit_key(&mut word, KeyCode::Backspace.into());
        assert!(word.is_empty());
    }

    #[test]
    fn editor_controls_do_not_change_the_word() {
        let mut word = "original".to_owned();
        assert_eq!(
            edit_key(&mut word, KeyCode::Enter.into()),
            Some(Input::Submit)
        );
        assert_eq!(
            edit_key(&mut word, KeyCode::Esc.into()),
            Some(Input::Cancel)
        );
        assert_eq!(
            edit_key(
                &mut word,
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
            ),
            Some(Input::Abort)
        );
        assert_eq!(word, "original");
    }
}
