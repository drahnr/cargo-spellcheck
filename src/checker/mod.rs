//! Checker
//!
//! Trait to handle additional trackers. Contains also helpers to avoid
//! re-implementing generic algorithms again and again, i.e. tokenization.

use crate::{CheckableChunk, Config, ContentOrigin, Detector, Suggestion};

use crate::errors::*;

mod cached;
use self::cached::Cached;

mod dictionary;
pub(crate) use self::dictionary::DictionaryReload;

use std::collections::HashSet;

mod tokenize;

#[cfg(feature = "hunspell")]
pub(crate) use self::hunspell::HunspellChecker;
#[cfg(feature = "nlprules")]
pub(crate) use self::nlprules::NlpRulesChecker;
#[cfg(feature = "spellbook")]
pub(crate) use self::spellbook::SpellbookChecker;
pub(crate) use self::tokenize::*;
#[cfg(feature = "zet")]
pub(crate) use self::zspell::ZetChecker;

#[cfg(feature = "hunspell")]
mod hunspell;

#[cfg(feature = "zet")]
mod zspell;

#[cfg(feature = "spellbook")]
mod spellbook;

#[cfg(feature = "nlprules")]
mod nlprules;

#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
pub(crate) mod dictaffix;

#[cfg(any(feature = "spellbook", feature = "zet", feature = "hunspell"))]
mod quirks;

#[cfg(test)]
pub mod dummy;

#[cfg(test)]
pub mod tests;

#[cfg(any(feature = "spellbook", feature = "zet", feature = "hunspell"))]
struct SuggestionContext<'a, 's> {
    plain: &'a crate::documentation::PlainOverlay<'s>,
    chunk: &'s CheckableChunk,
    origin: &'a ContentOrigin,
    allow_concatenated: bool,
    allow_dashed: bool,
    allow_emojis: bool,
}

#[cfg(any(feature = "spellbook", feature = "zet", feature = "hunspell"))]
impl<'s> SuggestionContext<'_, 's> {
    fn append_spelling_suggestions(
        &self,
        detector: Detector,
        word: String,
        range: crate::Range,
        replacements: Vec<String>,
        acc: &mut Vec<Suggestion<'s>>,
    ) {
        use quirks::{
            consists_of_vulgar_fractions_or_emojis, replacements_contain_dashed,
            replacements_contain_dashless,
        };

        if self.allow_emojis && consists_of_vulgar_fractions_or_emojis(&word) {
            log::trace!(target: "quirks", "Found emoji or vulgar fraction character, treating {word} as ok");
            return;
        }
        if self.allow_concatenated && replacements_contain_dashless(&word, &replacements) {
            log::trace!(target: "quirks", "Found dashless word in replacement suggestions, treating {word} as ok");
            return;
        }
        if self.allow_dashed && replacements_contain_dashed(&word, &replacements) {
            log::trace!(target: "quirks", "Found dashed word in replacement suggestions, treating {word} as ok");
            return;
        }
        for (range, span) in self.plain.find_spans(range) {
            acc.push(Suggestion {
                detector,
                checked_word: Some(word.clone()),
                range,
                span,
                origin: self.origin.clone(),
                replacements: replacements.clone(),
                chunk: self.chunk,
                description: Some("Possible spelling mistake found.".to_owned()),
            });
        }
    }
}

/// Implementation for a checker
pub trait Checker {
    type Config;

    fn detector() -> Detector;

    fn check<'a, 's>(
        &self,
        origin: &ContentOrigin,
        chunks: &'a [CheckableChunk],
    ) -> Result<Vec<Suggestion<'s>>>
    where
        'a: 's;
}

/// A normal spelling outcome, distinct from an operational/backend error.
///
/// Membership uses the rejecting detector as its payload. `Unchecked` means no
/// spelling engine is active; grammar-only checks never recognize vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SpellcheckResult<T> {
    Recognized,
    Unrecognized(T),
    Unchecked,
}

#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
impl<T> SpellcheckResult<T> {
    fn from_membership(recognized: bool, rejection: T) -> Self {
        if recognized {
            Self::Recognized
        } else {
            Self::Unrecognized(rejection)
        }
    }
}

// Keep construction concrete while sharing configuration gating between initial
// setup and prepare-before-install dictionary reloads.
macro_rules! create_checker {
    ($checker:ty, $config:expr, $field:ident) => {
        if $config.is_enabled(<$checker>::detector()) {
            log::debug!("Enabling {} checks.", <$checker>::detector());
            Some(<$checker>::new($config.$field.as_ref().unwrap())?)
        } else {
            None
        }
    };
}

/// Check a full document for violations using the tools we have.
///
/// Only configured checkers are used.
pub struct Checkers {
    config: Config,
    #[cfg(feature = "hunspell")]
    hunspell: Option<HunspellChecker>,
    #[cfg(feature = "zet")]
    zet: Option<ZetChecker>,
    #[cfg(feature = "spellbook")]
    spellbook: Option<SpellbookChecker>,
    #[cfg(feature = "nlprules")]
    nlprules: Option<NlpRulesChecker>,
}

impl Checkers {
    /// Test a literal word without Markdown/tokenization quirks. All active
    /// spelling backends must recognize it before learning can be skipped.
    /// Backend failures are errors, not ordinary unrecognized vocabulary.
    pub(crate) fn word_membership(
        &self,
        _word: &str,
    ) -> anyhow::Result<SpellcheckResult<Detector>> {
        let outcomes = std::iter::empty::<anyhow::Result<SpellcheckResult<Detector>>>();
        #[cfg(feature = "hunspell")]
        let outcomes = outcomes.chain(
            self.hunspell
                .iter()
                .map(|engine| engine.contains_word(_word)),
        );
        #[cfg(feature = "zet")]
        let outcomes = outcomes.chain(self.zet.iter().map(|engine| engine.contains_word(_word)));
        #[cfg(feature = "spellbook")]
        let outcomes = outcomes.chain(
            self.spellbook
                .iter()
                .map(|engine| engine.contains_word(_word)),
        );

        let mut recognized = false;
        for result in outcomes {
            let rejecting_detector = match result? {
                SpellcheckResult::Unrecognized(detector) => detector,
                SpellcheckResult::Recognized => {
                    recognized = true;
                    continue;
                }
                SpellcheckResult::Unchecked => continue,
            };
            return Ok(SpellcheckResult::Unrecognized(rejecting_detector));
        }
        Ok(if recognized {
            SpellcheckResult::Recognized
        } else {
            SpellcheckResult::Unchecked
        })
    }

    pub fn new(config: Config) -> Result<Self> {
        #[cfg(feature = "hunspell")]
        let hunspell = create_checker!(HunspellChecker, config, hunspell);
        #[cfg(feature = "zet")]
        let zet = create_checker!(ZetChecker, config, zet);
        #[cfg(feature = "spellbook")]
        let spellbook = create_checker!(SpellbookChecker, config, spellbook);
        #[cfg(feature = "nlprules")]
        let nlprules = create_checker!(NlpRulesChecker, config, nlprules);
        Ok(Self {
            config,
            #[cfg(feature = "hunspell")]
            hunspell,
            #[cfg(feature = "zet")]
            zet,
            #[cfg(feature = "spellbook")]
            spellbook,
            #[cfg(feature = "nlprules")]
            nlprules,
        })
    }
}

impl DictionaryReload for Checkers {
    fn include_learning_dictionary(&mut self, path: &std::path::Path) -> Result<()> {
        self.config.add_learning_dictionary(path);
        Ok(())
    }

    fn reload_dictionaries(&mut self) -> Result<()> {
        // Finish every fallible preparation before installing any replacement.
        // Learned vocabulary never changes grammar rules or their tokenizer.
        #[cfg(feature = "hunspell")]
        let hunspell = create_checker!(HunspellChecker, self.config, hunspell);
        #[cfg(feature = "zet")]
        let zet = create_checker!(ZetChecker, self.config, zet);
        #[cfg(feature = "spellbook")]
        let spellbook = create_checker!(SpellbookChecker, self.config, spellbook);

        #[cfg(feature = "hunspell")]
        {
            self.hunspell = hunspell;
        }
        #[cfg(feature = "zet")]
        {
            self.zet = zet;
        }
        #[cfg(feature = "spellbook")]
        {
            self.spellbook = spellbook;
        }
        Ok(())
    }

    fn verify_words(&self, words: &std::collections::BTreeSet<String>) -> Result<()> {
        for word in words {
            // Preserve the operational error and its sources across the existing
            // eyre-based persistence boundary shared with the learning coordinator.
            let outcome = self
                .word_membership(word)
                .map_err(|error| eyre!(error.into_boxed_dyn_error()))?;
            if let SpellcheckResult::Unrecognized(detector) = outcome {
                bail!("{detector} dictionary rejects accepted word {word:?} as a standalone word");
            }
        }
        Ok(())
    }
}

impl Checker for Checkers {
    type Config = Config;

    fn detector() -> Detector {
        unreachable!()
    }

    fn check<'a, 's>(
        &self,
        origin: &ContentOrigin,
        chunks: &'a [CheckableChunk],
    ) -> Result<Vec<Suggestion<'s>>>
    where
        'a: 's,
    {
        let mut collective = HashSet::<Suggestion<'s>>::new();
        #[cfg(feature = "hunspell")]
        if let Some(ref hunspell) = self.hunspell {
            collective.extend(hunspell.check(origin, chunks)?);
        }
        #[cfg(feature = "zet")]
        if let Some(ref zet) = self.zet {
            collective.extend(zet.check(origin, chunks)?);
        }
        #[cfg(feature = "spellbook")]
        if let Some(ref spellbook) = self.spellbook {
            collective.extend(spellbook.check(origin, chunks)?);
        }
        #[cfg(feature = "nlprules")]
        if let Some(ref nlprule) = self.nlprules {
            collective.extend(nlprule.check(origin, chunks)?);
        }

        Ok(normalize_suggestions(Vec::from_iter(collective)))
    }
}

fn normalize_suggestions(mut suggestions: Vec<Suggestion<'_>>) -> Vec<Suggestion<'_>> {
    // For equal spans, prefer checked-word metadata so deduplication retains
    // a representative that supports interactive acceptance, regardless of backend.
    suggestions.sort_by(|left, right| {
        left.cmp(right)
            .then_with(|| {
                right
                    .checked_word
                    .is_some()
                    .cmp(&left.checked_word.is_some())
            })
            .then_with(|| left.detector.as_str().cmp(right.detector.as_str()))
    });

    deduplicate_by_source(&mut suggestions);

    let mut retained = Vec::<Suggestion<'_>>::with_capacity(suggestions.len());
    for suggestion in suggestions {
        if retained
            .last()
            .is_some_and(|previous| suggestion.is_overlapped(previous))
        {
            continue;
        }
        retained.push(suggestion);
    }
    retained
}

/// Alternative chunks can report the same source text at different local ranges.
fn deduplicate_by_source(suggestions: &mut Vec<Suggestion<'_>>) {
    let mut seen = HashSet::new();
    suggestions.retain(|suggestion| {
        seen.insert((
            suggestion.origin.clone(),
            suggestion.detector,
            suggestion.span,
            suggestion.replacements.clone(),
            suggestion.description.clone(),
        ))
    });
}
