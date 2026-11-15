use super::*;
use std::convert::TryInto;

impl State<'_, '_> {
    pub(in crate::action) fn confirmed_plain(&self) -> UserSelection {
        assert!(self.can_accept);
        UserSelection::Nop.with_learning(learning::LearningOutcome::Confirmed(
            super::super::learning::AffixSuggestion {
                entry: DictionaryEntry::plain(self.suggestion.checked_word.as_deref().unwrap())
                    .unwrap(),
                forms: vec![self.suggestion.checked_word.clone().unwrap()],
            },
        ))
    }
}

fn chunk(text: &str) -> CheckableChunk {
    CheckableChunk::from_str(
        text,
        indexmap::indexmap! {
            0..text.chars().count() => (1_usize, 0..text.chars().count()).try_into().unwrap()
        },
        CommentVariant::TripleSlash,
    )
}

fn suggestion(chunk: &CheckableChunk, range: Range, detector: Detector) -> Suggestion<'_> {
    Suggestion {
        detector,
        origin: ContentOrigin::TestEntityRust,
        span: (1_usize, range.clone()).try_into().unwrap(),
        checked_word: if detector == Detector::Hunspell {
            Some(sub_chars(chunk.as_str(), range.clone()))
        } else {
            None
        },
        range,
        replacements: vec![],
        chunk,
        description: None,
    }
}

#[test]
fn feedback_is_one_replaceable_line_cleared_before_the_next_finding() {
    let mut output = Vec::new();
    {
        let mut feedback = FeedbackLine::new(&mut output);
        feedback.show().unwrap();
        feedback.show().unwrap();
        feedback.clear().unwrap();
        feedback.clear().unwrap();
    }
    write!(output, "next finding").unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\x1b[1G\x1b[2Knext up...\x1b[1G\x1b[2Knext up...\x1b[1G\x1b[2Knext finding"
    );
}

#[test]
fn feedback_is_cleared_on_termination_and_errors() {
    fn finish(output: &mut Vec<u8>, fail: bool) -> Result<()> {
        let mut feedback = FeedbackLine::new(output);
        feedback.show()?;
        if fail {
            return Err(crate::errors::eyre!("input failed"));
        }
        Ok(())
    }
    for fail in [false, true] {
        let mut output = Vec::new();
        assert_eq!(finish(&mut output, fail).is_err(), fail);
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\x1b[1G\x1b[2Knext up...\x1b[1G\x1b[2K"
        );
    }
}

#[test]
fn unused_feedback_does_not_touch_the_terminal() {
    let mut output = Vec::new();
    {
        let mut feedback = FeedbackLine::new(&mut output);
        feedback.clear().unwrap();
    }
    assert!(output.is_empty());
}

#[test]
fn shortcut_help_is_only_brief_context_commands() {
    for custom in [false, true] {
        for can_accept in [false, true] {
            let mut output = Vec::new();
            print_resolution_commands(&mut output, custom, can_accept).unwrap();
            let text = String::from_utf8(output).unwrap();
            assert!(text.starts_with("\r\n"));
            assert!(text.ends_with("? - show commands\r\n"));
            assert!(!text.contains('\x1b'));
            let mut keys = std::collections::BTreeSet::new();
            for line in text.trim_matches(['\r', '\n']).split("\r\n") {
                let (key, description) = line.split_once(" - ").unwrap();
                assert!(keys.insert(key), "duplicate command: {key}");
                assert!(!description.is_empty());
                assert!(description.split_whitespace().count() <= 6);
                assert!(!description.contains(['.', ';']));
            }
            assert_eq!(keys.contains("a"), !custom && can_accept);
            assert_eq!(keys.contains("Left"), custom);
            assert!(!keys.contains("g"));
            assert!(!keys.contains("J"));
        }
    }
}

#[test]
fn question_mark_never_edits_custom_input() {
    let chunk = chunk("Tokoi");
    let suggestion = suggestion(&chunk, 0..5, Detector::Hunspell);
    let mut state = State::new(&suggestion, true);
    state.select_custom();
    state.custom_replacement = "Tokio".into();
    state.cursor_offset = 2;
    let selected = state.pick_idx;
    assert_eq!(
        UserPicked::default()
            .enter_custom_replacement(&mut state, KeyCode::Char('?').into())
            .unwrap(),
        UserSelection::Nop
    );
    assert_eq!(state.custom_replacement, "Tokio");
    assert_eq!(state.cursor_offset, 2);
    assert_eq!(state.pick_idx, selected);
}

#[test]
fn selection_keys_navigate_and_wrap() {
    for count in [1, 3] {
        let mut selected = 0;
        for (key, expected) in [
            (KeyCode::Up, 1 % count),
            (KeyCode::Down, 0),
            (KeyCode::Down, count - 1),
            (KeyCode::Home, 0),
            (KeyCode::End, count - 1),
            (KeyCode::Up, 0),
        ] {
            assert_eq!(
                select_key(&mut selected, count, key.into()),
                Some(SelectionAction::Redraw)
            );
            assert_eq!(selected, expected);
        }
        assert_eq!(
            select_key(&mut selected, count, KeyCode::Char('e').into()),
            None
        );
        assert_eq!(selected, 0);
    }
}

#[test]
fn selection_keys_return_actions_without_moving() {
    let mut selected = 1;
    for (key, expected) in [
        (KeyCode::Enter.into(), SelectionAction::Submit),
        (KeyCode::Char('y').into(), SelectionAction::Submit),
        (KeyCode::Char('n').into(), SelectionAction::Skip),
        (KeyCode::Esc.into(), SelectionAction::Cancel),
        (
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            SelectionAction::Abort,
        ),
    ] {
        assert_eq!(select_key(&mut selected, 3, key), Some(expected));
        assert_eq!(selected, 1);
    }
}

#[test]
fn acceptance_suppresses_queued_duplicates_and_later_documents() {
    let chunk = chunk("Tokio Tokio");
    let mut session = SessionResolutions::default();
    let suggestions = vec![
        suggestion(&chunk, 0..5, Detector::Hunspell),
        suggestion(&chunk, 6..11, Detector::Hunspell),
    ];
    let mut prompts = 0;
    let (picked, outcome) = UserPicked::select_with(
        ContentOrigin::TestEntityRust,
        suggestions,
        &mut session,
        |_, state, _, _, _| {
            prompts += 1;
            Ok(state.confirmed_plain())
        },
    )
    .unwrap();
    assert_eq!(prompts, 1);
    assert!(picked.is_empty());
    assert_eq!(outcome, DocumentOutcome::Complete);
    assert!(session.accepts(&suggestion(&chunk, 0..5, Detector::Hunspell)));

    let later_chunk = self::chunk("Tokio");
    UserPicked::select_with(
        ContentOrigin::TestEntityRust,
        vec![suggestion(&later_chunk, 0..5, Detector::Hunspell)],
        &mut session,
        |_, _, _, _, _| panic!("accepted word prompted again in a later document"),
    )
    .unwrap();
}

#[test]
fn declined_learning_keeps_only_the_independent_source_choice() {
    let chunk = chunk("Tokoi");
    let suggestion = suggestion(&chunk, 0..5, Detector::Hunspell);
    let replacement = UserSelection::Replacement(BandAid::from(("Tokio".into(), &suggestion.span)));
    for selection in [UserSelection::Nop, replacement] {
        assert_eq!(
            selection
                .clone()
                .with_learning(learning::LearningOutcome::Declined),
            selection
        );
        assert_eq!(
            selection.with_learning(learning::LearningOutcome::Aborted),
            UserSelection::Abort
        );
    }
}

#[test]
fn confirmed_entries_only_suppress_their_spelling_forms() {
    for replace_source in [false, true] {
        let chunk = chunk("Tokoi Tokio Tokios");
        let mut session = SessionResolutions::default();
        let suggestions = vec![
            suggestion(&chunk, 0..5, Detector::Hunspell),
            suggestion(&chunk, 0..5, Detector::Hunspell),
            suggestion(&chunk, 6..11, Detector::Hunspell),
            suggestion(&chunk, 12..18, Detector::Hunspell),
            suggestion(&chunk, 6..11, Detector::NlpRules),
            suggestion(&chunk, 6..11, Detector::Reflow),
        ];
        let mut prompted = Vec::new();
        let (picked, outcome) = UserPicked::select_with(
            ContentOrigin::TestEntityRust,
            suggestions,
            &mut session,
            |_, state, idx, _, session| {
                prompted.push(idx);
                if idx != 0 {
                    assert_eq!(state.can_accept, idx < 4);
                    return Ok(UserSelection::Skip);
                }
                let selection = if replace_source {
                    UserSelection::Replacement(BandAid::from((
                        "Tokio".into(),
                        &state.suggestion.span,
                    )))
                } else {
                    UserSelection::Nop
                };
                Ok(
                    selection.with_learning(learning::LearningOutcome::Confirmed(
                        session.preview_entry("Tokio", None)?,
                    )),
                )
            },
        )
        .unwrap();
        assert_eq!(prompted, [0, 1, 3, 4, 5]);
        assert_eq!(picked.total_count(), usize::from(replace_source));
        assert_eq!(outcome, DocumentOutcome::Complete);
        assert!(session.knows_word("Tokio"));
        assert!(!session.knows_word("Tokoi"));
        assert!(!session.knows_word("Tokios"));
    }
}

#[test]
fn skipping_a_word_does_not_suppress_its_next_occurrence() {
    let chunk = chunk("Tokio Tokio");
    let mut session = SessionResolutions::default();
    let mut prompts = 0;
    let (picked, outcome) = UserPicked::select_with(
        ContentOrigin::TestEntityRust,
        vec![
            suggestion(&chunk, 0..5, Detector::Hunspell),
            suggestion(&chunk, 6..11, Detector::Hunspell),
        ],
        &mut session,
        |_, _, _, _, _| {
            prompts += 1;
            Ok(UserSelection::Skip)
        },
    )
    .unwrap();
    assert_eq!(prompts, 2);
    assert_eq!(outcome, DocumentOutcome::Complete);
    assert!(picked.is_empty());
    assert!(!session.accepts(&suggestion(&chunk, 0..5, Detector::Hunspell)));
}

#[test]
fn replacement_choices_do_not_suppress_or_learn_the_original_word() {
    for custom in [false, true] {
        let chunk = chunk("Tokoi Tokoi");
        let mut session = SessionResolutions::default();
        let mut first = suggestion(&chunk, 0..5, Detector::Hunspell);
        first.replacements = vec!["Tokio".into()];
        let mut prompts = 0;
        let (picked, _) = UserPicked::select_with(
            ContentOrigin::TestEntityRust,
            vec![first, suggestion(&chunk, 6..11, Detector::Hunspell)],
            &mut session,
            |_, state, _, _, _| {
                prompts += 1;
                Ok(if prompts == 1 {
                    if custom {
                        state.select_custom();
                        state.custom_replacement = "Tokio".into();
                    }
                    UserSelection::Replacement(state.to_bandaid())
                } else {
                    UserSelection::Skip
                })
            },
        )
        .unwrap();
        assert_eq!(prompts, 2);
        assert_eq!(picked.total_count(), 1);
        assert_eq!(
            picked.bandaids[&ContentOrigin::TestEntityRust][0].content,
            "Tokio"
        );
        assert!(!session.accepts(&suggestion(&chunk, 0..5, Detector::Hunspell)));
    }
}

#[test]
fn skip_file_returns_current_picks_without_prompting_remaining_suggestions() {
    let chunk = chunk("Tokoi Tokio Other");
    let mut session = SessionResolutions::default();
    let mut prompts = 0;
    let (picked, outcome) = UserPicked::select_with(
        ContentOrigin::TestEntityRust,
        vec![
            suggestion(&chunk, 0..5, Detector::Hunspell),
            suggestion(&chunk, 6..11, Detector::Hunspell),
            suggestion(&chunk, 12..17, Detector::Hunspell),
        ],
        &mut session,
        |_, state, _, _, _| {
            prompts += 1;
            Ok(match prompts {
                1 => {
                    state.select_custom();
                    state.custom_replacement = "Tokio".into();
                    UserSelection::Replacement(state.to_bandaid())
                }
                2 => UserSelection::SkipFile,
                _ => panic!("prompted another suggestion in the skipped file"),
            })
        },
    )
    .unwrap();
    assert_eq!(prompts, 2);
    assert_eq!(picked.total_count(), 1);
    assert_eq!(outcome, DocumentOutcome::SkipFile);
    assert!(!session.accepts(&suggestion(&chunk, 0..5, Detector::Hunspell)));
}

#[test]
fn picker_indices_and_default_keep_learning_opt_in() {
    let chunk = chunk("Tokoi");
    for can_accept in [false, true] {
        for replacements in [vec![], vec!["Tokio"], vec!["Tokio", "Tokyo", "Toki"]] {
            let mut suggestion = suggestion(&chunk, 0..5, Detector::Hunspell);
            suggestion.replacements = replacements.into_iter().map(str::to_owned).collect();
            let mut state = State::new(&suggestion, can_accept);
            let custom = usize::from(can_accept);
            let backtick = custom + 1;
            let first_replacement = custom + 2;

            assert_eq!(state.can_accept, can_accept);
            assert_eq!(state.custom_index(), custom);
            assert_eq!(
                state.n_items,
                suggestion.replacements.len() + first_replacement
            );
            assert_eq!(
                state.pick_idx,
                if suggestion.replacements.is_empty() {
                    backtick
                } else {
                    first_replacement
                }
            );
            assert!(!state.is_custom_entry());
            assert!(!state.is_learning_entry());
            assert_eq!(state.is_ticked_entry(), suggestion.replacements.is_empty());
            assert!(state.custom_replacement.is_empty());
            assert_eq!(state.cursor_offset, 0);

            state.select_custom();
            assert_eq!(state.pick_idx, custom);
            assert!(state.is_custom_entry());
            assert!(!state.is_ticked_entry());
            assert!(!state.is_learning_entry());
        }
    }
}

#[test]
fn every_picker_row_maps_to_learning_or_its_own_replacement() {
    let chunk = chunk("Tokoi");
    for can_accept in [false, true] {
        for replacements in [vec![], vec!["Tokio", "Tokyo"]] {
            let mut suggestion = suggestion(&chunk, 0..5, Detector::Hunspell);
            suggestion.replacements = replacements.into_iter().map(str::to_owned).collect();
            let mut state = State::new(&suggestion, can_accept);
            state.custom_replacement = "custom word".into();
            let custom = usize::from(can_accept);

            for index in 0..state.n_items {
                state.pick_idx = index;
                let learning = can_accept && index == 0;
                assert_eq!(state.is_learning_entry(), learning);
                assert_eq!(state.is_custom_entry(), index == custom);
                assert_eq!(state.is_ticked_entry(), index == custom + 1);
                if learning {
                    continue;
                }

                let expected = if index == custom {
                    "custom word"
                } else if index == custom + 1 {
                    "`Tokoi`"
                } else {
                    &suggestion.replacements[index - custom - 2]
                };
                let bandaid = state.to_bandaid();
                assert_eq!(bandaid.content, expected);
                assert_eq!(bandaid.span, suggestion.span);
            }
        }
    }
}

#[test]
fn shared_renderer_edits_the_custom_row_for_spelling_and_grammar() {
    let chunk = chunk("Tokoi");
    for (detector, can_accept) in [(Detector::Hunspell, true), (Detector::NlpRules, false)] {
        let suggestion = suggestion(&chunk, 0..5, detector);
        let mut state = State::new(&suggestion, can_accept);
        state.select_custom();
        let mut rows = Vec::new();
        if can_accept {
            rows.push((ContentStyle::new(), "Tokoi", Some("learn")));
        }
        rows.push((ContentStyle::new(), "...", Some("custom input")));
        rows.push((ContentStyle::new(), "`Tokoi`", Some("wrap backticks")));
        let mut output = Vec::new();
        print_selection_list(
            &mut output,
            rows,
            state.pick_idx,
            Some(state.custom_index()),
        )
        .unwrap();
        let raw = String::from_utf8(output).unwrap();
        assert!(raw.contains("\x1b[2K\x1b[?25h\x1b[3G"), "{raw:?}");
        assert!(!raw.contains("\x1b[?25l"), "{raw:?}");
        assert!(raw.contains(&selection_style().apply("...").to_string()));
        assert_eq!(console::strip_ansi_codes(&raw).matches('»').count(), 1);
        assert_eq!(raw.matches("\x1b[1A").count(), state.n_items - 1);
        assert!(raw.starts_with("\x1b7\x1b[2K"));
        assert!(raw.ends_with("\x1b8"));
    }
}

#[test]
fn row_descriptions_stay_outside_the_highlight() {
    let rows = [
        (ContentStyle::new(), "Tokoi", Some("learn")),
        (ContentStyle::new(), "...", Some("custom input")),
        (ContentStyle::new(), "`Tokoi`", Some("wrap backticks")),
    ];
    for selected in 0..rows.len() {
        let mut output = Vec::new();
        print_selection_list(&mut output, rows, selected, Some(1)).unwrap();
        let raw = String::from_utf8(output).unwrap();
        for (index, (style, content, description)) in rows.iter().enumerate() {
            let description = description.unwrap();
            assert!(description.split_whitespace().count() <= 2);
            let style = if index == selected {
                selection_style()
            } else {
                *style
            };
            assert!(
                raw.contains(&format!(
                    "{}{:padding$}< {description}",
                    style.apply(content),
                    "",
                    padding = 7 - console::measure_text_width(content) + 1,
                )),
                "Description must follow the styled content's reset: {raw:?}"
            );
        }
    }
}

#[test]
fn hint_columns_use_display_width_including_unhinted_rows() {
    let rows = [
        (ContentStyle::new(), "é", Some("learn")),
        (ContentStyle::new(), "e\u{301}", Some("custom input")),
        (ContentStyle::new(), "界🦀", Some("wrap backticks")),
        (ContentStyle::new(), "long suggestion", None),
    ];
    let width = console::measure_text_width("long suggestion");
    for selected in 0..rows.len() {
        let mut output = Vec::new();
        print_selection_list(&mut output, rows, selected, None).unwrap();
        let raw = String::from_utf8(output).unwrap();
        for row in raw.split("\x1b[5G").skip(1) {
            let text = console::strip_ansi_codes(row);
            if let Some((before_hint, _)) = text.split_once("< ") {
                assert_eq!(
                    console::measure_text_width(before_hint),
                    width + 1,
                    "{raw:?}"
                );
            }
        }
        assert_eq!(console::strip_ansi_codes(&raw).matches("< ").count(), 3);
    }
}

#[test]
fn custom_replacement_does_not_accept_vocabulary() {
    let chunk = chunk("Tokoi");
    let suggestion = suggestion(&chunk, 0..5, Detector::Hunspell);
    let mut state = State::new(&suggestion, false);
    state.select_custom();
    state.custom_replacement = "Tokio and friends".into();
    assert_eq!(state.to_bandaid().content, "Tokio and friends");
    let session = SessionResolutions::default();
    assert!(!session.accepts(&suggestion));
}

#[test]
fn unknown_words_without_replacements_can_be_accepted() {
    let chunk = chunk("naïve");
    let mut session = SessionResolutions::default();
    UserPicked::select_with(
        ContentOrigin::TestEntityRust,
        vec![suggestion(&chunk, 0..5, Detector::Hunspell)],
        &mut session,
        |_, state, _, _, _| Ok(state.confirmed_plain()),
    )
    .unwrap();
    assert!(session.accepts(&suggestion(&chunk, 0..5, Detector::Hunspell)));
}

#[test]
fn invalid_acceptance_is_rejected_without_ending_the_session() {
    let chunk = chunk("two words");
    let mut session = SessionResolutions::default();
    let mut prompts = 0;
    UserPicked::select_with(
        ContentOrigin::TestEntityRust,
        vec![suggestion(&chunk, 0..9, Detector::Hunspell)],
        &mut session,
        |_, _, _, _, _| {
            prompts += 1;
            Ok(if prompts == 1 {
                UserSelection::RecordEntry {
                    entry: DictionaryEntry::plain("valid").unwrap(),
                    forms: vec!["valid".into()],
                    replacement: None,
                }
            } else {
                UserSelection::Skip
            })
        },
    )
    .unwrap();
    assert_eq!(prompts, 2);
    assert!(!session.accepts(&suggestion(&chunk, 0..9, Detector::Hunspell)));
}

#[test]
fn no_replacement_prompt_keeps_the_same_context_layout() {
    let chunk = chunk("Unknown");
    let mut suggestion = suggestion(&chunk, 0..7, Detector::Hunspell);
    suggestion.description = Some("Possible spelling mistake found.".into());
    let without = format!(
        "{}\n{}",
        suggestion,
        State::new(&suggestion, false).prompt_padding()
    );
    suggestion.replacements.push("Known".into());
    let with = format!(
        "{}\n{}",
        suggestion,
        State::new(&suggestion, false).prompt_padding()
    );
    assert_eq!(without.lines().count(), with.lines().count());
}

#[test]
fn extending_picks_preserves_earlier_patches_for_the_same_file() {
    let mut picked = UserPicked::default();
    let mut later = UserPicked::default();
    picked.add_bandaid(
        &ContentOrigin::TestEntityRust,
        BandAid::from(("first".to_owned(), &(1_usize, 0..5).try_into().unwrap())),
    );
    later.add_bandaid(
        &ContentOrigin::TestEntityRust,
        BandAid::from(("second".to_owned(), &(2_usize, 0..6).try_into().unwrap())),
    );
    picked.extend(later);
    assert_eq!(picked.total_count(), 2);
    assert_eq!(
        picked.bandaids[&ContentOrigin::TestEntityRust][0].content,
        "first"
    );
    assert_eq!(
        picked.bandaids[&ContentOrigin::TestEntityRust][1].content,
        "second"
    );
}
