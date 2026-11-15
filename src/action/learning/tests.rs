use super::*;
use crate::{BandAid, CheckableChunk, CommentVariant, ContentOrigin, Detector, Range};
use std::cell::RefCell;
use std::convert::TryInto;
use std::rc::Rc;

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
        checked_word: Some(crate::sub_chars(chunk.as_str(), range.clone())),
        range,
        replacements: vec![],
        chunk,
        description: None,
    }
}

#[test]
fn acceptance_is_case_sensitive_and_shared_across_spelling_backends() {
    let chunk = chunk("Tokio tokio");
    let mut session = SessionResolutions::default();
    session
        .accept(&suggestion(&chunk, 0..5, Detector::Spellbook))
        .unwrap();
    for detector in [Detector::Hunspell, Detector::Spellbook, Detector::ZSpell] {
        assert!(session.accepts(&suggestion(&chunk, 0..5, detector)));
        assert!(!session.accepts(&suggestion(&chunk, 6..11, detector)));
    }
    let mut grammar = suggestion(&chunk, 0..5, Detector::NlpRules);
    grammar.checked_word = None;
    assert!(!session.accepts(&grammar));
    assert!(!session.can_accept(&grammar));
    assert!(session.accept(&grammar).is_err());
    assert!(!SessionResolutions::default().accepts(&suggestion(&chunk, 0..5, Detector::Hunspell)));
}

#[test]
fn missing_checked_word_never_falls_back_to_the_source_fragment() {
    let chunk = chunk("Tokio");
    let mut suggestion = suggestion(&chunk, 0..5, Detector::Hunspell);
    suggestion.checked_word = None;
    let mut session = SessionResolutions::default();
    session.accepted_words.insert("Tokio".into());
    assert!(!session.accepts(&suggestion));
    let error = session.accept(&suggestion).unwrap_err();
    assert!(error.to_string().contains("original checked word"));
    assert_eq!(session.accepted_words.len(), 1);
}

#[test]
fn eligibility_and_acceptance_use_capabilities_not_backend_identity() {
    let chunk = chunk("Tokio");
    for detector in [
        Detector::Hunspell,
        Detector::Spellbook,
        Detector::ZSpell,
        Detector::Dummy,
    ] {
        for word in [None, Some("Tokio"), Some("two words"), Some("word/S")] {
            let mut suggestion = suggestion(&chunk, 0..5, detector);
            suggestion.checked_word = word.map(str::to_owned);
            let mut session = SessionResolutions::default();
            let eligible = session.can_accept(&suggestion);
            assert_eq!(eligible, word == Some("Tokio"));
            assert_eq!(eligible, session.accept(&suggestion).is_ok());
            assert_eq!(eligible, session.accepts(&suggestion));
        }
    }
}

#[test]
fn duplicate_acceptance_keeps_one_pending_word() {
    let chunk = chunk("Tokio");
    let suggestion = suggestion(&chunk, 0..5, Detector::Hunspell);
    let mut session = SessionResolutions::default();
    session.accept(&suggestion).unwrap();
    session.accept(&suggestion).unwrap();
    assert_eq!(session.accepted_words.len(), 1);
    assert!(session.accepts(&suggestion));
}

type Events = Rc<RefCell<Vec<&'static str>>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Failure {
    None,
    Source,
    Update,
    Read,
    Reload,
    Verify,
    LostUpdate,
}

struct Sources {
    events: Events,
    fail: Failure,
    changes: usize,
}

impl SourceUpdates for Sources {
    fn apply_source_changes(&mut self, changes: UserPicked) -> Result<()> {
        self.events.borrow_mut().push("source");
        self.changes = changes.total_count();
        if self.fail == Failure::Source {
            bail!("source update failed");
        }
        Ok(())
    }
}

struct Engines {
    events: Events,
    fail: Failure,
}

impl DictionaryReload for Engines {
    fn reload_dictionaries(&mut self) -> Result<()> {
        self.events.borrow_mut().push("reload");
        if self.fail == Failure::Reload {
            bail!("dictionary reload failed");
        }
        Ok(())
    }

    fn verify_words(&self, words: &BTreeSet<String>) -> Result<()> {
        self.events.borrow_mut().push("verify");
        assert_eq!(words, &BTreeSet::from(["Tokio".into()]));
        if self.fail == Failure::Verify {
            bail!("dictionary verification failed");
        }
        Ok(())
    }
}

struct Store {
    events: Events,
    words: RefCell<BTreeSet<String>>,
    fail: Failure,
}

impl DictionaryStore for Store {
    fn validate_word(&self, word: &str) -> Result<()> {
        if word == "StoreRejected" {
            bail!("word rejected by store policy");
        }
        dictionary::validate_word(word)
    }

    fn read(&self) -> Result<BTreeSet<String>> {
        self.events.borrow_mut().push("read");
        if self.fail == Failure::Read {
            bail!("dictionary read failed");
        }
        Ok(self.words.borrow().clone())
    }

    fn update(&self, words: &BTreeSet<String>) -> Result<()> {
        self.events.borrow_mut().push("update");
        if self.fail == Failure::Update {
            bail!("dictionary update failed");
        }
        if self.fail != Failure::LostUpdate {
            self.words.borrow_mut().extend(words.iter().cloned());
        }
        Ok(())
    }
}

fn pending_session(events: &Events, failure: Failure) -> SessionResolutions {
    SessionResolutions {
        accepted_words: BTreeSet::from(["Tokio".into()]),
        accepted_entries: BTreeSet::from([DictionaryEntry::plain("Tokio").unwrap()]),
        store: Some(Box::new(Store {
            events: Rc::clone(events),
            words: RefCell::default(),
            fail: failure,
        })),
        ..SessionResolutions::default()
    }
}

#[test]
fn commit_orders_source_update_dictionary_update_readback_and_engine_reload() {
    let events = Events::default();
    let session = pending_session(&events, Failure::None);
    let mut sources = Sources {
        events: Rc::clone(&events),
        fail: Failure::None,
        changes: 0,
    };
    let mut engines = Engines {
        events: Rc::clone(&events),
        fail: Failure::None,
    };
    let mut changes = UserPicked::default();
    changes.add_bandaid(
        &ContentOrigin::TestEntityRust,
        BandAid::from(("Tokio".to_owned(), &(1_usize, 0..5).try_into().unwrap())),
    );
    session.commit(changes, &mut sources, &mut engines).unwrap();
    assert_eq!(sources.changes, 1);
    assert_eq!(
        *events.borrow(),
        ["source", "update", "read", "reload", "verify"]
    );
}

#[test]
fn commit_stops_at_each_failing_interface() {
    for (failure, expected, message) in [
        (Failure::Source, vec!["source"], "source update failed"),
        (
            Failure::Update,
            vec!["source", "update"],
            "dictionary update failed",
        ),
        (
            Failure::Read,
            vec!["source", "update", "read"],
            "dictionary read failed",
        ),
        (
            Failure::Reload,
            vec!["source", "update", "read", "reload"],
            "dictionary reload failed",
        ),
        (
            Failure::Verify,
            vec!["source", "update", "read", "reload", "verify"],
            "dictionary verification failed",
        ),
        (
            Failure::LostUpdate,
            vec!["source", "update", "read"],
            "did not retain every accepted word",
        ),
    ] {
        let events = Events::default();
        let session = pending_session(&events, failure);
        let mut sources = Sources {
            events: Rc::clone(&events),
            fail: failure,
            changes: 0,
        };
        let mut engines = Engines {
            events: Rc::clone(&events),
            fail: failure,
        };
        let error = session
            .commit(UserPicked::default(), &mut sources, &mut engines)
            .unwrap_err();
        assert!(error.to_string().contains(message), "{error:?}");
        assert_eq!(*events.borrow(), expected);
    }
}

#[test]
fn session_only_commit_does_not_touch_dictionary_interfaces() {
    let events = Events::default();
    let chunk = chunk("Tokio");
    let mut session = SessionResolutions::default();
    session
        .accept(&suggestion(&chunk, 0..5, Detector::ZSpell))
        .unwrap();
    let mut sources = Sources {
        events: Rc::clone(&events),
        fail: Failure::None,
        changes: 0,
    };
    let mut engines = Engines {
        events: Rc::clone(&events),
        fail: Failure::Reload,
    };
    session
        .commit(UserPicked::default(), &mut sources, &mut engines)
        .unwrap();
    assert_eq!(*events.borrow(), ["source"]);
}

#[test]
fn no_pending_acceptances_does_not_touch_dictionary_interfaces() {
    let events = Events::default();
    let mut session = pending_session(&events, Failure::Update);
    session.accepted_words.clear();
    session.accepted_entries.clear();
    let mut sources = Sources {
        events: Rc::clone(&events),
        fail: Failure::None,
        changes: 0,
    };
    let mut engines = Engines {
        events: Rc::clone(&events),
        fail: Failure::Reload,
    };
    session
        .commit(UserPicked::default(), &mut sources, &mut engines)
        .unwrap();
    assert_eq!(*events.borrow(), ["source"]);
}

#[test]
fn dropping_pending_learning_never_updates_or_reloads_dictionaries() {
    let events = Events::default();
    drop(pending_session(&events, Failure::None));
    assert!(events.borrow().is_empty());
}

#[test]
fn configured_store_controls_both_eligibility_and_recording() {
    let events = Events::default();
    let mut session = pending_session(&events, Failure::None);
    let chunk = chunk("StoreRejected");
    let suggestion = suggestion(&chunk, 0..13, Detector::Spellbook);
    assert!(!session.can_accept(&suggestion));
    assert!(session
        .accept(&suggestion)
        .unwrap_err()
        .to_string()
        .contains("store policy"));
    assert!(!session.accepts(&suggestion));
}

#[test]
fn stem_suggestions_validate_policy_before_requiring_configuration() {
    let events = Events::default();
    let session = pending_session(&events, Failure::None);
    for word in ["", "42", "Tokio/S", "two words", "Tokio\nInjected"] {
        let error = session.stem_suggestions(word, "S").unwrap_err();
        assert_eq!(
            error.to_string(),
            DictionaryEntry::plain(word).unwrap_err().to_string()
        );
    }
    assert!(session
        .stem_suggestions("StoreRejected", "S")
        .unwrap_err()
        .to_string()
        .contains("store policy"));
    assert!(session
        .stem_suggestions("Tokios", "S")
        .unwrap_err()
        .to_string()
        .contains("no spelling configuration"));
    assert!(events.borrow().is_empty());
}

#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
mod affix_sessions {
    use super::*;
    use crate::config::HunspellConfig;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    const RULES: &str = "SET UTF-8\nSFX S Y 2\nSFX S 0 s .\nSFX S 0 's .\nSFX D Y 2\nSFX D 0 d e\nSFX D 0 ed [^e]\n";

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "cargo-spellcheck-learning-session-{}",
                uuid::Uuid::new_v4()
            ));
            fs_err::create_dir(&path).unwrap();
            let fixture = Self(fs_err::canonicalize(path).unwrap());
            fs_err::write(fixture.0.join("en_US.aff"), RULES).unwrap();
            fs_err::write(fixture.0.join("en_US.dic"), "1\nTest\n").unwrap();
            fs_err::write(fixture.0.join("learning.dic"), "1\nExisting\n").unwrap();
            fixture
        }

        fn sessions(&self) -> Vec<(&'static str, SessionResolutions)> {
            let target = fs_err::canonicalize(self.0.join("learning.dic")).unwrap();
            let spelling = HunspellConfig {
                search_dirs: vec![self.0.clone()].into(),
                skip_os_lookups: true,
                use_builtin: false,
                extra_dictionaries: vec![target.clone()],
                learning_dictionary: Some(target),
                ..Default::default()
            };
            [
                (cfg!(feature = "hunspell"), "Hunspell", Detector::Hunspell),
                (cfg!(feature = "zet"), "ZSpell", Detector::ZSpell),
                (
                    cfg!(feature = "spellbook"),
                    "Spellbook",
                    Detector::Spellbook,
                ),
            ]
            .into_iter()
            .filter(|(enabled, _, _)| *enabled)
            .map(|(_, name, detector)| {
                let mut config = Config::parse("").unwrap();
                config.hunspell = None;
                config.zet = None;
                config.spellbook = None;
                config.nlprules = None;
                match detector {
                    Detector::Hunspell => config.hunspell = Some(spelling.clone()),
                    Detector::ZSpell => config.zet = Some(spelling.clone()),
                    Detector::Spellbook => config.spellbook = Some(spelling.clone()),
                    _ => unreachable!(),
                }
                (name, SessionResolutions::new(&config).unwrap())
            })
            .collect()
        }

        fn snapshot(&self) -> BTreeMap<std::ffi::OsString, Vec<u8>> {
            fs_err::read_dir(&self.0)
                .unwrap()
                .map(|entry| {
                    let entry = entry.unwrap();
                    (entry.file_name(), fs_err::read(entry.path()).unwrap())
                })
                .collect()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs_err::remove_dir_all(&self.0);
        }
    }

    fn record_entries(session: &mut SessionResolutions) {
        session
            .record_entry(
                DictionaryEntry::plain("Tokio").unwrap(),
                &["Tokio's".into(), "Tokios".into()],
            )
            .unwrap();
        session
            .record_entry(DictionaryEntry::plain("Ferris").unwrap(), &[])
            .unwrap();
    }

    fn source_changes() -> UserPicked {
        let mut changes = UserPicked::default();
        changes.add_bandaid(
            &ContentOrigin::TestEntityRust,
            BandAid::from(("Tokio".to_owned(), &(1_usize, 0..5).try_into().unwrap())),
        );
        changes
    }

    fn sources(events: &Events) -> Sources {
        Sources {
            events: Rc::clone(events),
            fail: Failure::None,
            changes: 0,
        }
    }

    struct EntryEngines {
        events: Events,
    }

    impl DictionaryReload for EntryEngines {
        fn reload_dictionaries(&mut self) -> Result<()> {
            self.events.borrow_mut().push("reload");
            Ok(())
        }

        fn verify_words(&self, words: &BTreeSet<String>) -> Result<()> {
            self.events.borrow_mut().push("verify");
            assert_eq!(words, &BTreeSet::from(["Ferris".into(), "Tokio".into()]));
            Ok(())
        }
    }

    struct EntryStore {
        events: Events,
        lines: Rc<RefCell<BTreeSet<String>>>,
        replace_plain_with_flagged: bool,
    }

    impl DictionaryStore for EntryStore {
        fn validate_word(&self, word: &str) -> Result<()> {
            dictionary::validate_word(word)
        }

        fn read(&self) -> Result<BTreeSet<String>> {
            panic!("typed commits must read exact entries, not stems")
        }

        fn update(&self, _: &BTreeSet<String>) -> Result<()> {
            panic!("typed commits must update entries, not plain words")
        }

        fn read_entries(&self) -> Result<BTreeSet<String>> {
            self.events.borrow_mut().push("read_entries");
            Ok(self.lines.borrow().clone())
        }

        fn update_entries(&self, entries: &BTreeSet<DictionaryEntry>) -> Result<()> {
            self.events.borrow_mut().push("update_entries");
            assert_eq!(
                entries
                    .iter()
                    .map(DictionaryEntry::to_line)
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from(["Ferris".into(), "Tokio".into()])
            );
            self.lines.borrow_mut().extend(entries.iter().map(|entry| {
                if self.replace_plain_with_flagged && entry.word() == "Tokio" {
                    "Tokio/S".into()
                } else {
                    entry.to_line()
                }
            }));
            Ok(())
        }
    }

    #[test]
    fn suffix_selection_shortens_stems_and_previews_edits_without_accepting_them() {
        let fixture = Fixture::new();
        let before = fixture.snapshot();
        for (_, session) in fixture.sessions() {
            for (word, flags, stem, edited, forms) in [
                (
                    "Tokios",
                    "S",
                    "Tokio",
                    "Nova",
                    vec!["Nova", "Nova's", "Novas"],
                ),
                ("hoped", "D", "hope", "love", vec!["love", "loved"]),
                (
                    "cafés",
                    "S",
                    "café",
                    "café",
                    vec!["café", "café's", "cafés"],
                ),
            ] {
                assert!(session
                    .entry_suggestions(word)
                    .unwrap()
                    .iter()
                    .any(|choice| choice.entry.flags() == Some(flags)));
                let choices = session.stem_suggestions(word, flags).unwrap();
                let shortened = choices
                    .iter()
                    .find(|choice| choice.entry.word() == stem)
                    .unwrap();
                assert_eq!(shortened.entry.to_line(), format!("{stem}/{flags}"));
                assert!(shortened.forms.contains(&word.to_owned()));
                assert_eq!(
                    choices.last().unwrap().entry.to_line(),
                    format!("{word}/{flags}")
                );
                assert!(choices[..choices.len() - 1]
                    .iter()
                    .all(|choice| choice.forms.contains(&word.to_owned())));
                let preview = session.preview_entry(edited, Some(flags)).unwrap();
                assert_eq!(preview.entry.to_line(), format!("{edited}/{flags}"));
                assert_eq!(preview.forms, forms);
            }
            for flags in ["X", "SD", "", "S\nInjected"] {
                assert!(session.stem_suggestions("Tokios", flags).is_err());
            }
            assert!(session.accepted_words.is_empty());
            assert!(session.accepted_entries.is_empty());
            assert_eq!(fixture.snapshot(), before);
        }
    }

    #[test]
    fn flagged_entries_overlay_every_surface_and_commit_only_exact_stem_flags() {
        let fixture = Fixture::new();
        for (_, mut session) in fixture.sessions() {
            fs_err::write(fixture.0.join("learning.dic"), "1\nExisting\n").unwrap();
            let before = fixture.snapshot();
            let mut engines =
                crate::checker::Checkers::new(session.config.clone().unwrap()).unwrap();
            assert!(!engines.contains_word("Tokios"));
            let stem_chunk = chunk("Tokio");
            session
                .accept(&suggestion(&stem_chunk, 0..5, Detector::Spellbook))
                .unwrap();
            for (stem, flags) in [("Tokio", "S"), ("hope", "D")] {
                let preview = session.preview_entry(stem, Some(flags)).unwrap();
                session
                    .record_entry(preview.entry.clone(), &preview.forms)
                    .unwrap();
                session.record_entry(preview.entry, &preview.forms).unwrap();
            }
            let forms = BTreeSet::from([
                "Tokio".into(),
                "Tokio's".into(),
                "Tokios".into(),
                "hope".into(),
                "hoped".into(),
            ]);
            assert_eq!(session.accepted_words, forms);
            for word in &forms {
                let chunk = chunk(word);
                for detector in [Detector::Hunspell, Detector::Spellbook, Detector::ZSpell] {
                    let suggestion = suggestion(&chunk, 0..word.chars().count(), detector);
                    assert!(session.accepts(&suggestion));
                    session.accept(&suggestion).unwrap();
                }
                session
                    .record_entry(DictionaryEntry::plain(word).unwrap(), &[])
                    .unwrap();
            }
            for word in ["tokio", "tokios", "Tokio/S", "Tokioed", "hopes"] {
                assert!(!session.knows_word(word));
            }
            assert_eq!(
                session
                    .accepted_entries
                    .iter()
                    .map(DictionaryEntry::to_line)
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from(["Tokio/S".into(), "hope/D".into()])
            );
            assert_eq!(fixture.snapshot(), before);
            let events = Events::default();
            session
                .commit(source_changes(), &mut sources(&events), &mut engines)
                .unwrap();
            assert_eq!(*events.borrow(), ["source"]);
            engines.verify_words(&forms).unwrap();
            let store = dictionary::DictionaryFile::new(fixture.0.join("learning.dic"));
            assert_eq!(
                store.read_entries().unwrap(),
                BTreeSet::from(["Existing".into(), "Tokio/S".into(), "hope/D".into()])
            );
            assert_eq!(
                fs_err::read_to_string(fixture.0.join("learning.dic")).unwrap(),
                "3\nExisting\nTokio/S\nhope/D\n"
            );
            for path in ["en_US.aff", "en_US.dic"] {
                assert_eq!(
                    fs_err::read(fixture.0.join(path)).unwrap(),
                    before[std::ffi::OsStr::new(path)]
                );
            }
        }
    }

    #[test]
    fn flagged_confirmation_requires_the_complete_regenerated_expansion() {
        let fixture = Fixture::new();
        let before = fixture.snapshot();
        for (_, mut session) in fixture.sessions() {
            record_entries(&mut session);
            let words = session.accepted_words.clone();
            let entries = session.accepted_entries.clone();
            for (stem, flags) in [("Tokio", "S"), ("hope", "D")] {
                let preview = session.preview_entry(stem, Some(flags)).unwrap();
                let mut forged = preview.forms.clone();
                forged.push("Unconfirmed".into());
                for forms in [vec![], preview.forms[1..].to_vec(), forged] {
                    let error = session
                        .record_entry(preview.entry.clone(), &forms)
                        .unwrap_err();
                    assert!(
                        error
                            .to_string()
                            .contains("do not match the regenerated expansion"),
                        "{error:?}"
                    );
                    assert_eq!(session.accepted_words, words);
                    assert_eq!(session.accepted_entries, entries);
                    assert_eq!(fixture.snapshot(), before);
                }
            }
        }
    }

    #[test]
    fn recording_reloads_changed_affix_definitions_instead_of_trusting_previews() {
        let fixture = Fixture::new();
        for (_, mut session) in fixture.sessions() {
            fs_err::write(fixture.0.join("en_US.aff"), RULES).unwrap();
            session.entry_suggestions("Tokios").unwrap();
            session.stem_suggestions("Tokios", "S").unwrap();
            let preview = session.preview_entry("Tokio", Some("S")).unwrap();
            for (rules, message) in [
                (
                    RULES.replace("SFX S 0 s .", "SFX S 0 z ."),
                    "do not match the regenerated expansion",
                ),
                (
                    "SET UTF-8\nSFX D Y 1\nSFX D 0 ed .\n".into(),
                    "Unknown or nonproductive suffix flag",
                ),
            ] {
                fs_err::write(fixture.0.join("en_US.aff"), rules).unwrap();
                let before = fixture.snapshot();
                let error = session
                    .record_entry(preview.entry.clone(), &preview.forms)
                    .unwrap_err();
                assert!(error.to_string().contains(message), "{error:?}");
                assert!(session.accepted_words.is_empty());
                assert!(session.accepted_entries.is_empty());
                assert_eq!(fixture.snapshot(), before);
            }
        }
    }

    #[test]
    fn commit_rejects_changed_affix_forms_before_any_source_or_dictionary_writes() {
        let fixture = Fixture::new();
        for rules in [
            RULES.replace("SFX S Y 2", "SFX S Y 3\nSFX S 0 z ."),
            RULES.replace("SFX S 0 s .", "SFX S 0 z ."),
            RULES.replace("SFX D Y 2", "SFX D Y 3\nSFX D 0 ful ."),
            "SET UTF-8\nSFX D Y 1\nSFX D 0 ed .\n".into(),
        ] {
            for (_, mut session) in fixture.sessions() {
                fs_err::write(fixture.0.join("en_US.aff"), RULES).unwrap();
                // These separately accepted words must not mask changes to an entry's own forms.
                for word in ["Tokioz", "hopeful"] {
                    session
                        .record_entry(DictionaryEntry::plain(word).unwrap(), &[])
                        .unwrap();
                }
                for (stem, flags) in [("Tokio", "S"), ("hope", "D")] {
                    let preview = session.preview_entry(stem, Some(flags)).unwrap();
                    session.record_entry(preview.entry, &preview.forms).unwrap();
                }
                fs_err::write(fixture.0.join("en_US.aff"), &rules).unwrap();
                let before = fixture.snapshot();
                let events = Events::default();
                let mut sources = sources(&events);
                let mut engines = Engines {
                    events: Rc::clone(&events),
                    fail: Failure::None,
                };
                let error = session
                    .commit(source_changes(), &mut sources, &mut engines)
                    .unwrap_err();
                assert!(
                    error.to_string().contains("preview the entry again"),
                    "{error:?}"
                );
                assert!(events.borrow().is_empty(), "no commit interface may run");
                assert_eq!(sources.changes, 0);
                assert_eq!(fixture.snapshot(), before);
            }
        }
    }

    #[test]
    fn commit_allows_affix_changes_that_preserve_confirmed_forms() {
        let fixture = Fixture::new();
        for (_, mut session) in fixture.sessions() {
            fs_err::write(fixture.0.join("en_US.aff"), RULES).unwrap();
            fs_err::write(fixture.0.join("learning.dic"), "1\nExisting\n").unwrap();
            let preview = session.preview_entry("Tokio", Some("S")).unwrap();
            session.record_entry(preview.entry, &preview.forms).unwrap();
            fs_err::write(
                fixture.0.join("en_US.aff"),
                RULES.replace("SFX D 0 ed [^e]", "SFX D 0 ing [^e]"),
            )
            .unwrap();
            let mut engines =
                crate::checker::Checkers::new(session.config.clone().unwrap()).unwrap();
            let events = Events::default();
            session
                .commit(source_changes(), &mut sources(&events), &mut engines)
                .unwrap();
            assert_eq!(*events.borrow(), ["source"]);
            let store = dictionary::DictionaryFile::new(fixture.0.join("learning.dic"));
            assert_eq!(
                store.read_entries().unwrap(),
                BTreeSet::from(["Existing".into(), "Tokio/S".into()])
            );
            engines
                .verify_words(&preview.forms.into_iter().collect())
                .unwrap();
        }
    }

    #[test]
    fn flagged_recording_validates_every_surface_against_store_policy_before_mutating() {
        let fixture = Fixture::new();
        fs_err::write(
            fixture.0.join("en_US.aff"),
            "SET UTF-8\nSFX S Y 1\nSFX S 0 Rejected .\n",
        )
        .unwrap();
        let before = fixture.snapshot();
        for (_, mut session) in fixture.sessions() {
            let events = Events::default();
            session.store = Some(Box::new(Store {
                events: Rc::clone(&events),
                words: RefCell::default(),
                fail: Failure::None,
            }));
            let preview = session.preview_entry("Store", Some("S")).unwrap();
            assert!(preview.forms.contains(&"StoreRejected".into()));
            let error = session
                .record_entry(preview.entry, &preview.forms)
                .unwrap_err();
            assert!(error.to_string().contains("store policy"), "{error:?}");
            assert!(session.accepted_words.is_empty());
            assert!(session.accepted_entries.is_empty());
            assert!(events.borrow().is_empty());
            assert_eq!(fixture.snapshot(), before);
        }
    }

    #[test]
    fn flagged_commit_stops_at_each_boundary_and_rejects_plain_stem_readback() {
        struct FlaggedStore {
            file: dictionary::DictionaryFile,
            events: Events,
            fail: Failure,
        }
        impl DictionaryStore for FlaggedStore {
            fn validate_word(&self, word: &str) -> Result<()> {
                self.file.validate_word(word)
            }
            fn read(&self) -> Result<BTreeSet<String>> {
                panic!("flagged commits must read exact entries")
            }
            fn update(&self, _: &BTreeSet<String>) -> Result<()> {
                panic!("flagged commits must not persist generated surfaces")
            }
            fn update_entries(&self, entries: &BTreeSet<DictionaryEntry>) -> Result<()> {
                self.events.borrow_mut().push("update");
                if self.fail == Failure::Update {
                    bail!("dictionary update failed");
                }
                assert_eq!(
                    entries
                        .iter()
                        .map(DictionaryEntry::to_line)
                        .collect::<Vec<_>>(),
                    ["Tokio/S"]
                );
                self.file.update_entries(entries)
            }
            fn read_entries(&self) -> Result<BTreeSet<String>> {
                self.events.borrow_mut().push("read");
                if self.fail == Failure::Read {
                    bail!("dictionary read failed");
                }
                if self.fail == Failure::LostUpdate {
                    return self.file.read();
                }
                self.file.read_entries()
            }
        }
        struct FlaggedEngines {
            inner: Engines,
            forms: BTreeSet<String>,
        }
        impl DictionaryReload for FlaggedEngines {
            fn reload_dictionaries(&mut self) -> Result<()> {
                self.inner.reload_dictionaries()
            }
            fn verify_words(&self, words: &BTreeSet<String>) -> Result<()> {
                self.inner.events.borrow_mut().push("verify");
                assert_eq!(words, &self.forms);
                if self.inner.fail == Failure::Verify {
                    bail!("dictionary verification failed");
                }
                Ok(())
            }
        }

        let fixture = Fixture::new();
        for (failure, expected, message) in [
            (Failure::Source, vec!["source"], "source update failed"),
            (
                Failure::Update,
                vec!["source", "update"],
                "dictionary update failed",
            ),
            (
                Failure::Read,
                vec!["source", "update", "read"],
                "dictionary read failed",
            ),
            (
                Failure::LostUpdate,
                vec!["source", "update", "read"],
                "did not retain every accepted word or confirmed entry",
            ),
            (
                Failure::Reload,
                vec!["source", "update", "read", "reload"],
                "dictionary reload failed",
            ),
            (
                Failure::Verify,
                vec!["source", "update", "read", "reload", "verify"],
                "dictionary verification failed",
            ),
        ] {
            for (_, mut session) in fixture.sessions() {
                fs_err::write(fixture.0.join("learning.dic"), "1\nExisting\n").unwrap();
                let events = Events::default();
                session.store = Some(Box::new(FlaggedStore {
                    file: dictionary::DictionaryFile::new(fixture.0.join("learning.dic")),
                    events: Rc::clone(&events),
                    fail: failure,
                }));
                let preview = session.preview_entry("Tokio", Some("S")).unwrap();
                session.record_entry(preview.entry, &preview.forms).unwrap();
                let mut engines = FlaggedEngines {
                    inner: Engines {
                        events: Rc::clone(&events),
                        fail: failure,
                    },
                    forms: preview.forms.into_iter().collect(),
                };
                let mut sources = sources(&events);
                sources.fail = failure;
                let error = session
                    .commit(source_changes(), &mut sources, &mut engines)
                    .unwrap_err();
                assert!(error.to_string().contains(message), "{error:?}");
                assert_eq!(*events.borrow(), expected);
            }
        }
    }

    #[test]
    fn unavailable_discovery_names_each_backend_without_mutating_session_or_files() {
        let fixture = Fixture::new();
        fs_err::remove_file(fixture.0.join("en_US.aff")).unwrap();
        let before = fixture.snapshot();
        for (backend, mut session) in fixture.sessions() {
            assert!(session.is_persistent());
            session
                .record_entry(DictionaryEntry::plain("Ferris").unwrap(), &[])
                .unwrap();
            let accepted_before = session.accepted_words.clone();
            let error = session.entry_suggestions("Tokio").err().unwrap();
            let message = error.to_string().to_lowercase();
            assert!(message.contains("unavailable"), "{error:?}");
            assert!(message.contains(&backend.to_lowercase()), "{error:?}");
            assert_eq!(session.accepted_words, accepted_before);
            assert!(!session.knows_word("Tokio"));
            assert_eq!(fixture.snapshot(), before);

            session
                .record_entry(DictionaryEntry::plain("Tokio").unwrap(), &[])
                .unwrap();
            assert!(session.knows_word("Ferris"));
            assert!(session.knows_word("Tokio"));
            drop(session);
            assert_eq!(fixture.snapshot(), before);
        }
    }

    #[test]
    fn discovery_rejects_dictionary_syntax_before_reporting_unavailable() {
        let fixture = Fixture::new();
        let before = fixture.snapshot();
        for (_, session) in fixture.sessions() {
            for word in [
                "",
                "42",
                "#comment",
                "Tokio/S",
                "two words",
                "Tokio\nInjected",
            ] {
                let error = session.entry_suggestions(word).err().unwrap();
                assert_eq!(
                    error.to_string(),
                    DictionaryEntry::plain(word).unwrap_err().to_string()
                );
            }
            assert!(session.accepted_words.is_empty());
            assert_eq!(fixture.snapshot(), before);
        }
    }

    #[test]
    fn flagged_recording_is_unavailable_without_mutating_session_or_files() {
        let fixture = Fixture::new();
        fs_err::remove_file(fixture.0.join("en_US.aff")).unwrap();
        let before = fixture.snapshot();
        let mut sessions = fixture.sessions();
        sessions.push(("session-only", SessionResolutions::default()));
        for (_, mut session) in sessions {
            record_entries(&mut session);
            let accepted_before = session.accepted_words.clone();
            for word in ["Novel", "Tokio"] {
                for forms in [vec![], vec![format!("{word}s"), format!("{word}'s")]] {
                    let error = session
                        .record_entry(DictionaryEntry::with_test_flags(word, "S").unwrap(), &forms)
                        .unwrap_err();
                    assert!(
                        error.to_string().to_lowercase().contains("unavailable"),
                        "{error:?}"
                    );
                    assert_eq!(session.accepted_words, accepted_before);
                    for form in forms {
                        assert!(!session.knows_word(&form));
                    }
                }
            }
            assert!(!session.knows_word("Novel"));
            assert_eq!(fixture.snapshot(), before);
            drop(session);
            assert_eq!(fixture.snapshot(), before);
        }
    }

    #[test]
    fn record_entry_checks_affix_capability_before_store_policy() {
        let events = Events::default();
        let mut session = pending_session(&events, Failure::None);
        let accepted_before = session.accepted_words.clone();
        let error = session
            .record_entry(
                DictionaryEntry::with_test_flags("StoreRejected", "S").unwrap(),
                &[],
            )
            .unwrap_err();
        assert!(
            error.to_string().to_lowercase().contains("unavailable"),
            "{error:?}"
        );
        let error = session
            .record_entry(DictionaryEntry::plain("StoreRejected").unwrap(), &[])
            .unwrap_err();
        assert!(error.to_string().contains("store policy"), "{error:?}");
        assert_eq!(session.accepted_words, accepted_before);
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn plain_entries_suppress_only_exact_words_across_backends_without_writes() {
        let fixture = Fixture::new();
        let before = fixture.snapshot();
        for (_, mut session) in fixture.sessions() {
            record_entries(&mut session);
            record_entries(&mut session);
            assert_eq!(
                session.accepted_words,
                BTreeSet::from(["Ferris".into(), "Tokio".into()])
            );
            for detector in [Detector::Hunspell, Detector::Spellbook, Detector::ZSpell] {
                for (word, accepted) in [
                    ("Ferris", true),
                    ("Tokio", true),
                    ("Tokio's", false),
                    ("Tokios", false),
                    ("tokio", false),
                    ("tokios", false),
                    ("Tokio/S", false),
                    ("Tokioed", false),
                ] {
                    let chunk = chunk(word);
                    let suggestion = suggestion(&chunk, 0..word.chars().count(), detector);
                    assert_eq!(
                        session.accepts(&suggestion),
                        accepted,
                        "{detector:?}: {word}"
                    );
                    assert_eq!(session.knows_word(word), accepted);
                    assert!(!SessionResolutions::default().accepts(&suggestion));
                }
            }
            assert_eq!(fixture.snapshot(), before);
            drop(session);
            assert_eq!(fixture.snapshot(), before);
        }
    }

    #[test]
    fn plain_recording_without_configuration_remains_session_only() {
        let events = Events::default();
        let mut session = SessionResolutions::default();
        assert!(!session.is_persistent());
        let error = session.entry_suggestions("Tokio").err().unwrap();
        assert!(error.to_string().contains("unavailable"), "{error:?}");
        record_entries(&mut session);
        assert!(session.knows_word("Tokio"));
        assert!(session.knows_word("Ferris"));
        assert!(!session.knows_word("Tokios"));
        let mut sources = sources(&events);
        let mut engines = EntryEngines {
            events: Rc::clone(&events),
        };
        session
            .commit(source_changes(), &mut sources, &mut engines)
            .unwrap();
        assert_eq!(sources.changes, 1);
        assert_eq!(*events.borrow(), ["source"]);
    }

    #[test]
    fn commit_persists_exact_plain_entries_before_reloading_and_verifying_only_plain_words() {
        let fixture = Fixture::new();
        for (_, mut session) in fixture.sessions() {
            let events = Events::default();
            let lines = Rc::new(RefCell::new(BTreeSet::from(["Existing".into()])));
            session.store = Some(Box::new(EntryStore {
                events: Rc::clone(&events),
                lines: Rc::clone(&lines),
                replace_plain_with_flagged: false,
            }));
            record_entries(&mut session);
            assert!(events.borrow().is_empty());
            assert_eq!(*lines.borrow(), BTreeSet::from(["Existing".into()]));
            let mut sources = sources(&events);
            let mut engines = EntryEngines {
                events: Rc::clone(&events),
            };
            session
                .commit(source_changes(), &mut sources, &mut engines)
                .unwrap();
            assert_eq!(sources.changes, 1);
            assert_eq!(
                *events.borrow(),
                [
                    "source",
                    "update_entries",
                    "read_entries",
                    "reload",
                    "verify"
                ]
            );
            assert_eq!(
                *lines.borrow(),
                BTreeSet::from(["Existing".into(), "Ferris".into(), "Tokio".into()])
            );
        }
    }

    #[test]
    fn unavailable_discovery_does_not_prevent_plain_dictionary_commit() {
        let fixture = Fixture::new();
        fs_err::write(fixture.0.join("en_US.aff"), "SET UTF-8\nFLAG invalid\n").unwrap();
        for (_, mut session) in fixture.sessions() {
            fs_err::write(fixture.0.join("learning.dic"), "1\nExisting\n").unwrap();
            let events = Events::default();
            assert!(session.entry_suggestions("Tokio").is_err());
            record_entries(&mut session);
            let mut sources = sources(&events);
            let mut engines = EntryEngines {
                events: Rc::clone(&events),
            };
            session
                .commit(source_changes(), &mut sources, &mut engines)
                .unwrap();
            assert_eq!(sources.changes, 1);
            assert_eq!(*events.borrow(), ["source", "reload", "verify"]);
            let store = dictionary::DictionaryFile::new(fixture.0.join("learning.dic"));
            assert_eq!(
                store.read_entries().unwrap(),
                BTreeSet::from(["Existing".into(), "Ferris".into(), "Tokio".into()])
            );
            assert_eq!(
                fs_err::read_to_string(fixture.0.join("en_US.aff")).unwrap(),
                "SET UTF-8\nFLAG invalid\n"
            );
            assert_eq!(
                fs_err::read_to_string(fixture.0.join("en_US.dic")).unwrap(),
                "1\nTest\n"
            );
        }
    }

    #[test]
    fn readback_rejects_flagged_substitution_for_plain_word_before_reload() {
        let fixture = Fixture::new();
        for (_, mut session) in fixture.sessions() {
            let events = Events::default();
            let lines = Rc::new(RefCell::new(BTreeSet::new()));
            session.store = Some(Box::new(EntryStore {
                events: Rc::clone(&events),
                lines: Rc::clone(&lines),
                replace_plain_with_flagged: true,
            }));
            record_entries(&mut session);
            let mut sources = sources(&events);
            let mut engines = EntryEngines {
                events: Rc::clone(&events),
            };
            let error = session
                .commit(source_changes(), &mut sources, &mut engines)
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("did not retain every accepted word or confirmed entry"),
                "{error:?}"
            );
            assert_eq!(sources.changes, 1);
            assert_eq!(
                *lines.borrow(),
                BTreeSet::from(["Ferris".into(), "Tokio/S".into()])
            );
            assert_eq!(
                *events.borrow(),
                ["source", "update_entries", "read_entries"]
            );
        }
    }
}
