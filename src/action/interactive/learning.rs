//! Read-only learning prompts; the caller owns recording and persistence.

use super::{
    print_commands, print_selection_list, select_key, selection_style, ScopedRaw, SelectionAction,
    NAVIGATION_COMMANDS,
};
use crate::action::learning::{AffixPattern, AffixSuggestion, DictionaryEntry, SessionResolutions};
use crate::errors::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    style::{Color, ContentStyle, PrintStyledContent, SetForegroundColor, Stylize},
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
    Ok(choose_and_confirm(word, session)?.unwrap_or(LearningOutcome::Cancel))
}

fn print_entry_preview(
    output: &mut impl Write,
    entry: &DictionaryEntry,
    forms: &[String],
) -> Result<()> {
    write!(output, "New entry: ")?;
    output.queue(PrintStyledContent(selection_style().apply(entry.to_line())))?;
    write!(output, "\r\nWould match: ")?;
    if entry.flags().is_none() {
        write!(
            output,
            "{} (plain word; no affixes)\r\n",
            highlight_stem(entry.word())
        )?;
    } else if forms.is_empty() {
        write!(
            output,
            "unavailable (backend supplied no expanded forms)\r\n"
        )?;
    } else {
        write!(output, "{}\r\n", highlight_expansion(entry.word(), forms))?;
    }
    Ok(())
}

fn highlight_stem(stem: &str) -> String {
    if stem.is_empty() {
        String::new()
    } else {
        stem.with(Color::Cyan).to_string()
    }
}

fn highlight_expansion(stem: &str, forms: &[String]) -> String {
    forms
        .iter()
        .map(|form| {
            let boundaries = stem
                .char_indices()
                .map(|(index, _)| index)
                .chain(std::iter::once(stem.len()))
                .collect::<Vec<_>>();
            let mut core = (0, 0);
            'search: for length in (1..boundaries.len()).rev() {
                for start in 0..boundaries.len() - length {
                    let candidate = &stem[boundaries[start]..boundaries[start + length]];
                    if let Some(offset) = form.find(candidate) {
                        core = (offset, offset + candidate.len());
                        break 'search;
                    }
                }
            }
            let highlight = |text: &str| {
                if text.is_empty() {
                    String::new()
                } else {
                    selection_style().apply(text).to_string()
                }
            };
            format!(
                "{}{}{}",
                highlight(&form[..core.0]),
                highlight_stem(&form[core.0..core.1]),
                highlight(&form[core.1..])
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn print_learning_prompt(output: &mut impl Write, word: &str, accept_as_is: bool) -> Result<()> {
    print_entry_preview(output, &DictionaryEntry::plain(word)?, &[])?;
    print_learning_options(output, word, accept_as_is)
}

fn print_learning_options(output: &mut impl Write, word: &str, accept_as_is: bool) -> Result<()> {
    let replacement = if accept_as_is { "" } else { " replacement" };
    write!(output, "Learn{replacement} \"")?;
    output.queue(PrintStyledContent(selection_style().apply(word)))?;
    write!(output, "\" [p,e,n,?]: ")?;
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
    let mut patterns = match session.pattern_suggestions(word) {
        Ok(patterns) => patterns,
        Err(error) => {
            write!(
                stdout(),
                "{error}; plain-word learning is still available.\r\n"
            )?;
            Vec::new()
        }
    };
    patterns.push(AffixPattern {
        prefix: None,
        suffix: None,
        suggestion: session.preview_entry(word, None)?,
    });
    let mut selected = 0;
    loop {
        match choose_pattern(&mut patterns, &mut selected, session)? {
            PatternAction::Manual => return choose_manually(word, session),
            PatternAction::Cancel => return Ok(None),
            PatternAction::Abort => return Ok(Some(LearningOutcome::Abort)),
            PatternAction::Submit => {
                match confirm_entry(&patterns[selected].suggestion, session, false)? {
                    EditOutcome::Back => continue,
                    EditOutcome::Finished(outcome) => return Ok(outcome),
                }
            }
        }
    }
}

fn choose_manually(word: &str, session: &SessionResolutions) -> Result<Option<LearningOutcome>> {
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
            EntryAction::Abort => return Ok(Some(LearningOutcome::Abort)),
            EntryAction::Cancel => return Ok(None),
            EntryAction::Edit => match confirm_entry(&suffixes[selected], session, true)? {
                EditOutcome::Back => continue 'suffix,
                EditOutcome::Finished(outcome) => return Ok(outcome),
            },
            EntryAction::Submit => {}
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
                    EntryAction::Abort => return Ok(Some(LearningOutcome::Abort)),
                    EntryAction::Cancel => continue 'suffix,
                    EntryAction::Submit | EntryAction::Edit => selected,
                }
            } else {
                0
            };
            match confirm_entry(&stems[selected], session, true)? {
                EditOutcome::Back if flags.is_some() => continue 'stem,
                EditOutcome::Back => continue 'suffix,
                EditOutcome::Finished(outcome) => return Ok(outcome),
            }
        }
    }
}

enum EditOutcome {
    Back,
    Finished(Option<LearningOutcome>),
}

fn edit_stem(
    suggestion: &AffixSuggestion,
    session: &SessionResolutions,
) -> Result<(Input, AffixSuggestion)> {
    let mut stem = suggestion.entry.word().to_owned();
    loop {
        let (action, edited) = read_line(EditorStage::Stem, &stem)?;
        match action {
            Input::Abort | Input::Cancel => return Ok((action, suggestion.clone())),
            Input::Submit => stem = edited,
        }
        match session.preview_entry(&stem, suggestion.entry.flags()) {
            Ok(preview) => return Ok((Input::Submit, preview)),
            Err(error) => write!(stdout(), "Invalid stem: {error}\r\n")?,
        }
    }
}

fn confirm_entry(
    suggestion: &AffixSuggestion,
    session: &SessionResolutions,
    mut edit_first: bool,
) -> Result<EditOutcome> {
    let mut preview = suggestion.clone();
    'edit: loop {
        if edit_first {
            let (action, edited) = edit_stem(&preview, session)?;
            match action {
                Input::Abort => return Ok(EditOutcome::Finished(Some(LearningOutcome::Abort))),
                Input::Cancel => return Ok(EditOutcome::Back),
                Input::Submit => preview = edited,
            }
        }
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
                return Ok(EditOutcome::Finished(Some(LearningOutcome::Abort)));
            }
            match key.code {
                KeyCode::Enter | KeyCode::Char('y')
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    write!(stdout(), "\r\n")?;
                    return Ok(EditOutcome::Finished(Some(LearningOutcome::Record(
                        preview.entry,
                        preview.forms,
                    ))));
                }
                KeyCode::Char('e') => {
                    write!(stdout(), "\r\n")?;
                    edit_first = true;
                    continue 'edit;
                }
                KeyCode::Esc | KeyCode::Char('n') => {
                    write!(stdout(), "\r\n")?;
                    return Ok(EditOutcome::Finished(None));
                }
                _ => {}
            }
        }
    }
}

const AFFIX_PROMPT: &str = "Affixes [y,a,e,n,?]: ";
const NO_AFFIX: &str = "Ø";

enum PatternAction {
    Submit,
    Manual,
    Cancel,
    Abort,
}

fn sorted_extensions(session: &SessionResolutions, stem: &str) -> Result<Vec<AffixPattern>> {
    let mut choices = session.extension_patterns(stem)?;
    choices.sort_by(|left, right| {
        left.suggestion
            .entry
            .flags()
            .unwrap_or("")
            .cmp(right.suggestion.entry.flags().unwrap_or(""))
    });
    Ok(choices)
}

fn extension_label(pattern: &AffixPattern) -> String {
    pattern
        .suggestion
        .entry
        .flags()
        .map_or_else(|| NO_AFFIX.to_owned(), |flags| format!("/{flags}"))
}

fn extension_strip(labels: &[String], selected: usize, width: u16) -> (String, String) {
    let width = usize::from(width.saturating_sub(1));
    if labels.is_empty() || width == 0 {
        return (String::new(), String::new());
    }
    let selected_label = &labels[selected];
    let selected_label = if console::measure_text_width(selected_label) <= width {
        std::borrow::Cow::Borrowed(selected_label.as_str())
    } else {
        console::truncate_str(selected_label, width, "…")
    };
    let caret = width / 2;
    let selected_width = console::measure_text_width(&selected_label);
    let selected_start = caret.saturating_sub(selected_width / 2);

    // Keep the chosen label centered and move whole neighboring labels beneath
    // the caret. Reserve edge markers whenever more alternatives are hidden.
    let mut start = selected;
    let mut left_width = 0;
    while start > 0 {
        let candidate = left_width + console::measure_text_width(&labels[start - 1]) + 1;
        let marker = if start > 1 { 3 } else { 0 };
        if candidate + marker > selected_start {
            break;
        }
        start -= 1;
        left_width = candidate;
    }
    let mut left = if start > 0 && selected_start >= left_width + 3 {
        ".. ".to_owned()
    } else {
        String::new()
    };
    for label in &labels[start..selected] {
        left.push_str(label);
        left.push(' ');
    }
    let padding = selected_start.saturating_sub(console::measure_text_width(&left));
    let mut strip = format!("{:padding$}{left}{selected_label}", "");
    let mut used = selected_start + selected_width;
    let mut end = selected + 1;
    while end < labels.len() {
        let candidate = used + 1 + console::measure_text_width(&labels[end]);
        let marker = if end + 1 < labels.len() { 3 } else { 0 };
        if candidate + marker > width {
            break;
        }
        strip.push(' ');
        strip.push_str(&labels[end]);
        used = candidate;
        end += 1;
    }
    if end < labels.len() && used + 3 <= width {
        strip.push_str(" ..");
    }
    (
        strip,
        format!("{:selected_start$}{}", "", "^".repeat(selected_width)),
    )
}

fn highlight_extension_strip(strip: &str) -> String {
    strip
        .split(' ')
        .map(|label| match label {
            "" | ".." => label.to_owned(),
            NO_AFFIX => label.with(Color::Yellow).to_string(),
            _ => selection_style().apply(label).to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn highlight_extension_marker(marker: &str) -> String {
    let padding = marker.len() - marker.trim_start().len();
    let (padding, marker) = marker.split_at(padding);
    if marker.is_empty() {
        padding.to_owned()
    } else {
        format!("{padding}{}", marker.with(Color::Red))
    }
}

fn highlight_empty_affixes(row: &str, selected: bool) -> String {
    let cells = row.split('|').collect::<Vec<_>>();
    if cells.len() < 2 {
        return row.to_owned();
    }
    let stem_index = usize::from(cells.len() == 3);
    let foreground = if selected { Color::Green } else { Color::Blue };
    let empty = format!(
        "{}{NO_AFFIX}{}",
        SetForegroundColor(Color::Yellow),
        SetForegroundColor(foreground),
    );
    cells
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            if index != stem_index && cell.trim() == NO_AFFIX {
                // Restore the enclosing row's foreground without resetting its
                // selected background/bold attributes or coloring the padding.
                cell.replace(NO_AFFIX, &empty)
            } else {
                (*cell).to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn choose_pattern(
    patterns: &mut [AffixPattern],
    selected: &mut usize,
    session: &SessionResolutions,
) -> Result<PatternAction> {
    let mut output = stdout();
    let (width, height) = terminal::size().unwrap_or((80, 24));
    if height < 4 {
        return Err(crate::errors::eyre!(
            "Affix selection requires at least four terminal rows; enlarge the terminal and retry"
        ));
    }
    let visible = patterns
        .len()
        .min(usize::from(height.saturating_sub(6).max(1)));
    let stem = patterns[*selected].suggestion.entry.word();
    let extensions = match sorted_extensions(session, stem) {
        Ok(extensions) => extensions,
        Err(error) => {
            write!(output, "{error}; extension cycling is unavailable.\r\n")?;
            Vec::new()
        }
    };
    // Flag availability is catalog-wide, including conditional rules that do
    // not currently change this stem. Keep those columns stable while cycling.
    let layout = extensions.clone();
    let mut cache = std::collections::BTreeMap::from([(stem.to_owned(), extensions)]);
    let (mut heading, _) = pattern_rows(patterns, &layout, width);
    mount_patterns(&mut output, &heading, visible)?;
    loop {
        let (next_heading, rows) = pattern_rows(patterns, &layout, width);
        heading = next_heading;
        let rows = rows
            .iter()
            .enumerate()
            .map(|(index, row)| highlight_empty_affixes(row, index == *selected))
            .collect::<Vec<_>>();
        let pattern = &patterns[*selected];
        let stem = pattern.suggestion.entry.word();
        let choices = match cache.entry(stem.to_owned()) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                let extensions = match sorted_extensions(session, stem) {
                    Ok(extensions) => extensions,
                    Err(error) => {
                        write!(
                            output,
                            "\r\n\r\n\r\n\r\n{error}; extension cycling is unavailable.\r\n"
                        )?;
                        mount_patterns(&mut output, &heading, visible)?;
                        Vec::new()
                    }
                };
                entry.insert(extensions)
            }
        };
        let start = selected.saturating_sub(visible - 1);
        let mut style = ContentStyle::new();
        style.background_color = Some(Color::Black);
        style.foreground_color = Some(Color::Blue);
        print_selection_list(
            &mut output,
            rows.iter()
                .zip(patterns.iter())
                .skip(start)
                .take(visible)
                .map(|(row, pattern)| {
                    (
                        style,
                        row.as_str(),
                        Some(if pattern.suggestion.entry.flags().is_some() {
                            "pattern"
                        } else {
                            "plain"
                        }),
                    )
                }),
            *selected - start,
            None,
        )?;
        let current = choices
            .iter()
            .position(|choice| choice.prefix == pattern.prefix && choice.suffix == pattern.suffix)
            .unwrap_or(0);
        let labels = if choices.is_empty() {
            vec![extension_label(pattern)]
        } else {
            choices.iter().map(extension_label).collect()
        };
        let (strip, caret) = extension_strip(&labels, current, width);
        let strip = highlight_extension_strip(&strip);
        let caret = highlight_extension_marker(&caret);
        print_picker_context(
            &mut output,
            &pattern.suggestion,
            width,
            Some((&strip, &caret)),
        )?;
        let key = read_key()?;
        if key.code == KeyCode::Char('?') {
            write!(output, "\r\n\r\n\r\n\r\n")?;
            print_commands(
                &mut output,
                NAVIGATION_COMMANDS.iter().copied().chain([
                    ("Left", "previous extension"),
                    ("Right", "next extension"),
                    ("Enter", "preview entry"),
                    ("y", "preview entry"),
                    ("a", "choose affixes"),
                    ("e", "edit stem"),
                    ("n", "cancel learning"),
                    ("Esc", "cancel learning"),
                    ("Ctrl-C", "discard choices and abort"),
                ]),
            )?;
            mount_patterns(&mut output, &heading, visible)?;
            continue;
        }
        if matches!(key.code, KeyCode::Left | KeyCode::Right) {
            if !choices.is_empty() {
                let next = if key.code == KeyCode::Left {
                    (current + choices.len() - 1) % choices.len()
                } else {
                    (current + 1) % choices.len()
                };
                patterns[*selected] = choices[next].clone();
            }
            continue;
        }
        if key.code == KeyCode::Char('e') {
            write!(output, "\r\n\r\n\r\n\r\n")?;
            let (action, suggestion) = edit_stem(&patterns[*selected].suggestion, session)?;
            match action {
                Input::Abort => return Ok(PatternAction::Abort),
                Input::Cancel => {}
                Input::Submit => patterns[*selected].suggestion = suggestion,
            }
            let (next_heading, _) = pattern_rows(patterns, &layout, width);
            mount_patterns(&mut output, &next_heading, visible)?;
            continue;
        }
        let action = if key.code == KeyCode::Char('a') {
            Some(PatternAction::Manual)
        } else {
            match select_entry_key(selected, patterns.len(), key) {
                Some(Input::Submit) => Some(PatternAction::Submit),
                Some(Input::Cancel) => Some(PatternAction::Cancel),
                Some(Input::Abort) => Some(PatternAction::Abort),
                None => None,
            }
        };
        if let Some(action) = action {
            write!(output, "\r\n\r\n\r\n\r\n")?;
            return Ok(action);
        }
    }
}

fn mount_patterns(output: &mut impl Write, heading: &str, visible: usize) -> Result<()> {
    write!(output, "{AFFIX_PROMPT}\r\n    {heading}\r\n")?;
    for _ in 0..visible + 3 {
        write!(output, "\r\n")?;
    }
    output.queue(cursor::MoveUp(3))?;
    Ok(())
}

fn pattern_rows(
    patterns: &[AffixPattern],
    extensions: &[AffixPattern],
    width: u16,
) -> (String, Vec<String>) {
    let with_prefix = patterns
        .iter()
        .chain(extensions)
        .any(|pattern| pattern.prefix.is_some());
    let flags = |flag: &Option<String>| {
        flag.as_ref()
            .map_or_else(|| NO_AFFIX.to_owned(), |flag| format!("/{flag}"))
    };
    let prefix_width = patterns
        .iter()
        .chain(extensions)
        .map(|pattern| console::measure_text_width(&flags(&pattern.prefix)))
        .max()
        .unwrap_or(0)
        .max(6);
    let suffix_width = patterns
        .iter()
        .chain(extensions)
        .map(|pattern| console::measure_text_width(&flags(&pattern.suffix)))
        .max()
        .unwrap_or(0)
        .max(6);
    let fixed = if with_prefix { prefix_width + 3 } else { 0 } + 3 + suffix_width;
    let row_width = usize::from(width.saturating_sub(15));
    let clip = |text: &str, width: usize| {
        if console::measure_text_width(text) <= width {
            text.to_owned()
        } else if width == 0 {
            String::new()
        } else {
            console::truncate_str(text, width, "…").into_owned()
        }
    };
    // If the fixed columns alone crowd out the stem, show a clipped native
    // entry instead. Wrapping a picker row would invalidate cursor restores.
    if fixed + 4 > row_width {
        return (
            clip("stem/flags", row_width),
            patterns
                .iter()
                .map(|pattern| clip(&pattern.suggestion.entry.to_line(), row_width))
                .collect(),
        );
    }
    let available = row_width - fixed;
    let stem_width = patterns
        .iter()
        .map(|pattern| console::measure_text_width(pattern.suggestion.entry.word()))
        .max()
        .unwrap_or(0)
        .max(4)
        .min(available);
    let row = |prefix: &str, stem: &str, suffix: &str| {
        let stem = if console::measure_text_width(stem) <= stem_width {
            std::borrow::Cow::Borrowed(stem)
        } else if stem_width == 0 {
            std::borrow::Cow::Borrowed("")
        } else {
            console::truncate_str(stem, stem_width, "…")
        };
        let padding = stem_width.saturating_sub(console::measure_text_width(&stem));
        if with_prefix {
            let prefix_padding = prefix_width.saturating_sub(console::measure_text_width(prefix));
            format!(
                "{prefix}{:prefix_padding$} | {stem}{:padding$} | {suffix}",
                "", ""
            )
        } else {
            format!("{stem}{:padding$} | {suffix}", "")
        }
    };
    let heading = row("prefix", "stem", "suffix");
    let rows = patterns
        .iter()
        .map(|pattern| {
            row(
                &flags(&pattern.prefix),
                pattern.suggestion.entry.word(),
                &flags(&pattern.suffix),
            )
        })
        .collect();
    (heading, rows)
}

const CONFIRMATION_PROMPT: &str = "Confirm learning [y,e,n,?]: ";

fn print_confirmation_commands(output: &mut impl Write) -> Result<()> {
    print_commands(
        output,
        [
            ("y", "record entry"),
            ("Enter", "record entry"),
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
            Self::Suffix => "Suffix [y,e,n,?]: ",
            Self::Stem => "Stem [y,e,n,?]: ",
        }
    }
}

enum EntryAction {
    Submit,
    Edit,
    Cancel,
    Abort,
}

fn choose_entry(choices: &[AffixSuggestion], stage: EntryStage) -> Result<(EntryAction, usize)> {
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
        let action = if key.code == KeyCode::Char('e') {
            Some(EntryAction::Edit)
        } else {
            select_entry_key(&mut selected, choices.len(), key).map(|action| match action {
                Input::Submit => EntryAction::Submit,
                Input::Cancel => EntryAction::Cancel,
                Input::Abort => EntryAction::Abort,
            })
        };
        if let Some(action) = action {
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
            ("e", "edit stem"),
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
            (content, description)
        })
        .collect::<Vec<_>>();
    let description_width = rows
        .iter()
        .map(|(_, description)| console::measure_text_width(description))
        .max()
        .unwrap_or(0);
    let available = width.saturating_sub(description_width + 3);
    let rows = rows
        .into_iter()
        .map(|(content, description)| {
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
    print_picker_context(output, suggestion, width, None)
}

fn print_picker_context(
    output: &mut impl Write,
    suggestion: &AffixSuggestion,
    width: u16,
    strip: Option<(&str, &str)>,
) -> Result<()> {
    let expansion = if suggestion.entry.flags().is_none() {
        format!("{} (plain word; no affixes)", suggestion.entry.word())
    } else {
        suggestion.forms.join(", ")
    };
    let context = format!("Would match: {expansion}");
    let visible = console::truncate_str(&context, usize::from(width.saturating_sub(1)), "…");
    // Clip plain text first so invisible extensions cannot leave stray styles.
    let rendered = if let Some(forms) = visible.strip_prefix("Would match: ") {
        let (forms, ellipsis) = if visible != context && forms.ends_with('…') {
            (forms.strip_suffix('…').unwrap(), "…")
        } else {
            (forms, "")
        };
        let matches = if suggestion.entry.flags().is_some() {
            let forms = forms.split(", ").map(str::to_owned).collect::<Vec<_>>();
            highlight_expansion(suggestion.entry.word(), &forms)
        } else {
            let stem_end = suggestion.entry.word().len().min(forms.len());
            format!(
                "{}{}",
                highlight_stem(&forms[..stem_end]),
                &forms[stem_end..]
            )
        };
        format!("Would match: {matches}{ellipsis}")
    } else {
        visible.into_owned()
    };
    output.queue(cursor::SavePosition)?;
    let rows = strip.into_iter().flat_map(|(strip, caret)| [strip, caret]);
    for row in rows.chain(std::iter::once(rendered.as_str())) {
        output
            .queue(cursor::MoveDown(1))?
            .queue(cursor::MoveToColumn(0))?
            .queue(terminal::Clear(terminal::ClearType::CurrentLine))?;
        write!(output, "{row}")?;
    }
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
            Self::Word => "Word [?]: ",
            Self::Stem => "Stem [?]: ",
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
            assert!(console::strip_ansi_codes(&raw).contains("\"MSRV\" [p,e,n,?]: "));
            assert!(raw.ends_with(" [p,e,n,?]: "));
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
            "\r\ny - record entry\r\nEnter - record entry\r\nn - decline learning\r\nEsc - decline learning\r\ne - edit stem\r\nCtrl-C - discard choices and abort\r\n? - show commands\r\n"
        );
        assert_eq!(CONFIRMATION_PROMPT, "Confirm learning [y,e,n,?]: ");
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
            assert_eq!(stage.prompt(), format!("{name} [?]: "));
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
                ("e", "edit stem"),
                ("n", back),
                ("Esc", back),
                ("Ctrl-C", "discard choices and abort"),
                ("?", "show commands"),
            ]) {
                assert!(help.contains(&format!("{key} - {effect}\r\n")));
            }
            assert!(help.contains(&format!("? - show commands\r\n{name} [")));
            assert_eq!(stage.prompt(), format!("{name} [y,e,n,?]: "));
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
    fn immediate_pattern_columns_align_unicode_and_keep_full_entries() {
        for with_prefix in [false, true] {
            let patterns = vec![
                AffixPattern {
                    prefix: with_prefix.then(|| "S".to_owned()),
                    suffix: (!with_prefix).then(|| "S".to_owned()),
                    suggestion: AffixSuggestion {
                        entry: DictionaryEntry::with_test_flags("café🦀", "S").unwrap(),
                        forms: vec![],
                    },
                },
                AffixPattern {
                    prefix: None,
                    suffix: None,
                    suggestion: AffixSuggestion {
                        entry: DictionaryEntry::plain("e\u{301}longoriginal").unwrap(),
                        forms: vec![],
                    },
                },
            ];
            for width in [40, 80] {
                let (heading, rows) = pattern_rows(&patterns, &[], width);
                let columns = |row: &str| {
                    row.match_indices('|')
                        .map(|(offset, _)| console::measure_text_width(&row[..offset]))
                        .collect::<Vec<_>>()
                };
                assert_eq!(columns(&heading).len(), if with_prefix { 2 } else { 1 });
                for row in &rows {
                    assert_eq!(columns(row), columns(&heading));
                    assert!(
                        console::measure_text_width(row) + 4 + " < pattern".len()
                            < usize::from(width)
                    );
                }
                let cells = rows[0].split('|').map(str::trim).collect::<Vec<_>>();
                if with_prefix {
                    assert_eq!(cells[0], "/S");
                } else {
                    assert_eq!(cells[1], "/S");
                }
            }
            let (_, rows) = pattern_rows(&patterns, &[], 80);
            assert!(rows[1].contains("e\u{301}longoriginal"));
            assert!(
                !rows[1].contains('…'),
                "Exact-width values must not be shortened"
            );
            for width in [0, 1] {
                let _ = pattern_rows(&patterns, &[], width);
            }
            assert_eq!(patterns[0].suggestion.entry.to_line(), "café🦀/S");
        }
    }

    #[test]
    fn extension_strip_centers_the_selection_and_scrolls_whole_sorted_labels() {
        let labels = std::iter::once(NO_AFFIX.to_owned())
            .chain(('A'..='Z').map(|flag| format!("/{flag}")))
            .collect::<Vec<_>>();
        for width in [16, 24, 40, 80, 160] {
            let mut previous = None;
            for (selected, label) in labels.iter().enumerate() {
                let (strip, caret) = extension_strip(&labels, selected, width);
                assert!(console::measure_text_width(&strip) < usize::from(width));
                let label_width = console::measure_text_width(label);
                assert_eq!(caret.trim(), "^".repeat(label_width));
                let column =
                    console::measure_text_width(caret.split_once('^').unwrap().0) + label_width / 2;
                assert_eq!(column, usize::from(width - 1) / 2);
                let tokens = strip
                    .split_whitespace()
                    .filter(|token| *token != "..")
                    .collect::<Vec<_>>();
                assert!(tokens.contains(&label.as_str()));
                assert!(tokens.windows(2).all(|pair| {
                    pair[0].strip_prefix('/').unwrap_or("")
                        < pair[1].strip_prefix('/').unwrap_or("")
                }));
                let selected_offset = strip
                    .match_indices(label)
                    .find_map(|(offset, _)| {
                        let after = &strip[offset + label.len()..];
                        after
                            .chars()
                            .next()
                            .is_none_or(char::is_whitespace)
                            .then_some(offset)
                    })
                    .unwrap();
                assert_eq!(
                    console::measure_text_width(&strip[..selected_offset])
                        + console::measure_text_width(label) / 2,
                    column
                );
                if let Some(previous) = previous {
                    assert_ne!(strip, previous);
                }
                previous = Some(strip);
            }
        }
    }

    #[test]
    fn empty_affixes_are_yellow_without_recoloring_stems_or_real_flags() {
        for selected in [false, true] {
            let foreground = if selected { Color::Green } else { Color::Blue };
            let empty = format!(
                "{}Ø{}",
                SetForegroundColor(Color::Yellow),
                SetForegroundColor(foreground)
            );
            assert_eq!(
                highlight_empty_affixes("Ø | stem | Ø", selected),
                format!("{empty} | stem | {empty}")
            );
            assert_eq!(
                highlight_empty_affixes("Ø | Ø", selected),
                format!("Ø | {empty}")
            );
            assert_eq!(
                highlight_empty_affixes("/Ø | stem | Ø", selected),
                format!("/Ø | stem | {empty}")
            );
            assert_eq!(highlight_empty_affixes("stem/Ø", selected), "stem/Ø");
        }
        let strip = "  Ø /S /Ø ..";
        assert_eq!(
            highlight_extension_strip(strip),
            format!(
                "  {} {} {} ..",
                "Ø".with(Color::Yellow),
                selection_style().apply("/S"),
                selection_style().apply("/Ø")
            ),
        );
        assert_eq!(
            highlight_extension_marker("  ^^^"),
            format!("  {}", "^^^".with(Color::Red))
        );
        assert_eq!(highlight_extension_marker(""), "");
    }

    #[test]
    fn extension_strip_handles_unicode_labels_and_extremely_narrow_terminals() {
        let labels = [NO_AFFIX, "/Ä", "/東京", "/🦀"].map(str::to_owned);
        for width in 0..80 {
            for selected in 0..labels.len() {
                let (strip, caret) = extension_strip(&labels, selected, width);
                assert!(
                    console::measure_text_width(&strip) <= usize::from(width.saturating_sub(1))
                );
                assert!(
                    console::measure_text_width(&caret) <= usize::from(width.saturating_sub(1))
                );
            }
        }
    }

    #[test]
    fn catalog_columns_stay_stable_and_narrow_rows_do_not_wrap() {
        let patterns = vec![AffixPattern {
            prefix: None,
            suffix: Some("S".to_owned()),
            suggestion: AffixSuggestion {
                entry: DictionaryEntry::with_test_flags("café🦀root", "S").unwrap(),
                forms: vec![],
            },
        }];
        let extensions = vec![AffixPattern {
            prefix: Some("LONGPREFIX".to_owned()),
            suffix: Some("LONGSUFFIX".to_owned()),
            suggestion: AffixSuggestion {
                entry: DictionaryEntry::with_test_flags("café🦀root", "S").unwrap(),
                forms: vec![],
            },
        }];
        let (heading, _) = pattern_rows(&patterns, &extensions, 80);
        assert!(heading.contains("prefix"));
        let (cycled_heading, _) = pattern_rows(&extensions, &extensions, 80);
        assert_eq!(heading, cycled_heading);
        for width in 16..32 {
            let (heading, rows) = pattern_rows(&patterns, &extensions, width);
            assert!(console::measure_text_width(&heading) + 4 < usize::from(width));
            for row in rows {
                assert!(
                    console::measure_text_width(&row) + 4 + " < pattern".len() < usize::from(width),
                    "row must not wrap at width {width}: {row}"
                );
            }
        }
    }

    #[test]
    fn prefix_expansions_color_the_stem_core_separately_from_affixes() {
        let forms = ["hope", "unhope", "hoped", "unhoped", "unhoping"].map(str::to_owned);
        assert_eq!(
            highlight_expansion("hope", &forms),
            format!(
                "{hope}, {}{hope}, {hope}{}, {}{hope}{}, {}{hop}{}",
                selection_style().apply("un"),
                selection_style().apply("d"),
                selection_style().apply("un"),
                selection_style().apply("d"),
                selection_style().apply("un"),
                selection_style().apply("ing"),
                hope = "hope".with(Color::Cyan),
                hop = "hop".with(Color::Cyan),
            )
        );
    }

    #[test]
    fn expansions_highlight_only_added_or_changed_affixes() {
        for (stem, form, prefix, extension) in [
            ("hope", "hoped", "hope", "d"),
            ("hope", "hoping", "hop", "ing"),
            ("city", "cities", "cit", "ies"),
            ("café", "cafés", "café", "s"),
            ("e\u{301}", "e\u{301}s", "e\u{301}", "s"),
        ] {
            let rendered = highlight_expansion(stem, &[stem.to_owned(), form.to_owned()]);
            assert_eq!(
                rendered,
                format!(
                    "{}, {}{}",
                    stem.with(Color::Cyan),
                    prefix.with(Color::Cyan),
                    selection_style().apply(extension)
                )
            );
            assert_eq!(
                console::strip_ansi_codes(&rendered),
                format!("{stem}, {form}")
            );
            assert!(rendered.ends_with("\x1b[0m"));
        }
        assert_eq!(
            highlight_expansion("same", &["same".to_owned()]),
            "same".with(Color::Cyan).to_string()
        );
    }

    #[test]
    fn aligned_suffix_hints_fit_within_the_available_columns() {
        let choices = vec![
            AffixSuggestion {
                entry: DictionaryEntry::plain("longplainword").unwrap(),
                forms: vec![],
            },
            AffixSuggestion {
                entry: DictionaryEntry::with_test_flags("stem", "S").unwrap(),
                forms: vec![],
            },
        ];
        let mut output = Vec::new();
        print_entries(&mut output, &choices, 0, 20, 10, EntryStage::Suffix).unwrap();
        let raw = String::from_utf8(output).unwrap();
        let mut columns = Vec::new();
        for row in raw.split("\x1b[5G").skip(1) {
            let text = console::strip_ansi_codes(row);
            let (content, hint) = text.split_once("< ").unwrap();
            // Ignore subsequent cursor commands and marker when counting this row.
            let hint = hint.split(['»']).next().unwrap();
            columns.push(console::measure_text_width(content));
            assert!(
                4 + console::measure_text_width(content) + 2 + console::measure_text_width(hint)
                    < 20
            );
        }
        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0], columns[1]);
    }

    #[test]
    fn plain_entry_preview_explicitly_has_no_affixes() {
        let mut output = Vec::new();
        print_entry_preview(&mut output, &DictionaryEntry::plain("MSRV").unwrap(), &[]).unwrap();
        let raw = String::from_utf8(output).unwrap();
        assert!(raw.contains(&selection_style().apply("MSRV").to_string()));
        assert_eq!(
            console::strip_ansi_codes(&raw),
            "New entry: MSRV\r\nWould match: MSRV (plain word; no affixes)\r\n"
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
        assert!(raw.contains(&format!(
            "Would match: {stem}, {stem}{}\r\n",
            selection_style().apply("s"),
            stem = "widget".with(Color::Cyan)
        )));
        assert_eq!(
            console::strip_ansi_codes(&raw),
            "New entry: widget/S\r\nWould match: widget, widgets\r\n"
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
            "New entry: widget/S\r\nWould match: unavailable (backend supplied no expanded forms)\r\n"
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
