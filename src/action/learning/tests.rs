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
        store: Some(Box::new(Store {
            events: Rc::clone(events),
            words: RefCell::default(),
            fail: failure,
        })),
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
