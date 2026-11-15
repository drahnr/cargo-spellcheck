//! Checker
//!
//! Trait to handle additional trackers. Contains also helpers to avoid
//! re-implementing generic algorithms again and again, i.e. tokenization.

use crate::{CheckableChunk, Config, ContentOrigin, Detector, Suggestion};

use crate::errors::*;

mod cached;
use self::cached::Cached;

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
mod dictaffix;

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

/// Check a full document for violations using the tools we have.
///
/// Only configured checkers are used.
pub struct Checkers {
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
    pub fn new(config: Config) -> Result<Self> {
        macro_rules! create_checker {
            ($feature:literal, $checker:ty, $config:expr, $checker_config:expr) => {
                if !cfg!(feature = $feature) {
                    log::debug!("Feature {} is disabled by compilation.", $feature);
                    None
                } else {
                    let config = $config;
                    let detector = <$checker>::detector();
                    if config.is_enabled(detector) {
                        log::debug!("Enabling {} checks.", detector);
                        Some(<$checker>::new($checker_config.unwrap())?)
                    } else {
                        log::debug!("Checker {detector} is disabled by configuration.");
                        None
                    }
                }
            };
        }

        #[cfg(feature = "hunspell")]
        let hunspell = create_checker!(
            "hunspell",
            HunspellChecker,
            &config,
            config.hunspell.as_ref()
        );
        #[cfg(feature = "zet")]
        let zet = create_checker!("zet", ZetChecker, &config, config.zet.as_ref());
        #[cfg(feature = "spellbook")]
        let spellbook = create_checker!(
            "spellbook",
            SpellbookChecker,
            &config,
            config.spellbook.as_ref()
        );
        #[cfg(feature = "nlprules")]
        let nlprules = create_checker!(
            "nlprules",
            NlpRulesChecker,
            &config,
            config.nlprules.as_ref()
        );
        Ok(Self {
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

        let mut suggestions: Vec<Suggestion<'s>> = Vec::from_iter(collective);
        suggestions.sort();
        deduplicate_by_source(&mut suggestions);
        if suggestions.is_empty() {
            return Ok(suggestions);
        }

        // Iterate through suggestions and identify overlapping ones.
        let mut retained = Vec::<Suggestion<'s>>::with_capacity(suggestions.len());
        for suggestion in suggestions {
            if retained
                .last()
                .is_some_and(|previous| suggestion.is_overlapped(previous))
            {
                continue;
            }
            retained.push(suggestion);
        }

        Ok(retained)
    }
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
