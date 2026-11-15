use super::*;
use std::convert::TryInto;

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
        |_, _, _, _| {
            prompts += 1;
            Ok(UserSelection::AcceptWord)
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
        |_, _, _, _| panic!("accepted word prompted again in a later document"),
    )
    .unwrap();
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
        |_, _, _, _| {
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
            |_, state, _, _| {
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
        |_, state, _, _| {
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
fn custom_replacement_does_not_accept_vocabulary() {
    let chunk = chunk("Tokoi");
    let suggestion = suggestion(&chunk, 0..5, Detector::Hunspell);
    let mut state = State::from(&suggestion);
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
        |_, _, _, _| Ok(UserSelection::AcceptWord),
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
        |_, _, _, _| {
            prompts += 1;
            Ok(if prompts == 1 {
                UserSelection::AcceptWord
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
        State::from(&suggestion).prompt_padding()
    );
    suggestion.replacements.push("Known".into());
    let with = format!(
        "{}\n{}",
        suggestion,
        State::from(&suggestion).prompt_padding()
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
