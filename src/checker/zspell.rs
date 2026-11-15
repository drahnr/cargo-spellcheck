//! A dictionary check with affixes, backed by `libhunspell`
//!
//! Does not check grammar, but tokenizes the documentation chunk, and checks
//! the individual tokens against the dictionary using the defined affixes. Can
//! handle multiple dictionaries.

use super::{apply_tokenizer, Checker, Detector, Suggestion, SuggestionContext};

use crate::checker::dictaffix::DicAff;
use crate::config::WrappedRegex;
use crate::documentation::{CheckableChunk, ContentOrigin};
use crate::util::sub_chars;
use crate::Range;

use nlprule::Tokenizer;

use std::path::PathBuf;
use std::sync::Arc;

use doc_chunks::Ignores;

use crate::errors::*;

use super::quirks::{
    compound_ranges, consists_of_vulgar_fractions_or_emojis, replacements_contain_dashed,
    replacements_contain_dashless, transform, Transformed,
};

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub struct ZetCheckerInner {
    zspell: zspell::Dictionary,
    transform_regex: Vec<WrappedRegex>,
    allow_concatenated: bool,
    allow_dashed: bool,
    allow_emojis: bool,
    check_footnote_references: bool,
    ignorelist: String,
}

impl ZetCheckerInner {
    fn new(config: &<ZetChecker as Checker>::Config) -> Result<Self> {
        // TODO allow override
        let (
            transform_regex,
            allow_concatenated,
            allow_dashed,
            allow_emojis,
            check_footnote_references,
        ) = {
            let quirks = &config.quirks;
            (
                quirks.transform_regex().to_vec(),
                quirks.allow_concatenated(),
                quirks.allow_dashed(),
                quirks.allow_emojis(),
                quirks.check_footnote_references(),
            )
        };
        // FIXME rename the config option
        let ignorelist = config.tokenization_splitchars.clone();
        // without these, a lot of those would be flagged as mistakes.
        debug_assert!(ignorelist.contains(','));
        debug_assert!(ignorelist.contains('.'));
        debug_assert!(ignorelist.contains(';'));
        debug_assert!(ignorelist.contains('!'));
        debug_assert!(ignorelist.contains('?'));

        let DicAff { dic, aff } = DicAff::load(
            &config.extra_dictionaries[..],
            &config.search_dirs,
            config.lang(),
            config.use_builtin,
            config.skip_os_lookups,
        )?;

        let aff = aff.replace("UTF8", "UTF-8");
        let zet = zspell::builder().config_str(&aff).dict_str(&dic).build()?;

        log::debug!("Dictionary setup completed successfully.");
        Ok(Self {
            zspell: zet,
            transform_regex,
            allow_concatenated,
            allow_dashed,
            allow_emojis,
            check_footnote_references,
            ignorelist,
        })
    }
}

#[derive(Clone)]
pub struct ZetChecker(pub Arc<ZetCheckerInner>, pub Arc<Tokenizer>);

impl std::ops::Deref for ZetChecker {
    type Target = ZetCheckerInner;
    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl ZetChecker {
    pub fn new(config: &<ZetChecker as Checker>::Config) -> Result<Self> {
        let tokenizer = super::tokenizer::<&PathBuf>(None)?;
        let inner = ZetCheckerInner::new(config)?;
        let hunspell = Arc::new(inner);
        Ok(ZetChecker(hunspell, tokenizer))
    }

    pub(super) fn contains_word(&self, word: &str) -> bool {
        self.zspell.check_word(word)
    }
}

impl Checker for ZetChecker {
    type Config = crate::config::ZetConfig;

    fn detector() -> Detector {
        Detector::ZSpell
    }

    fn check<'a, 's>(
        &self,
        origin: &ContentOrigin,
        chunks: &'a [CheckableChunk],
    ) -> Result<Vec<Suggestion<'s>>>
    where
        'a: 's,
    {
        let mut acc = Vec::with_capacity(chunks.len());

        for chunk in chunks {
            let plain = chunk.erase_cmark(&Ignores {
                footnote_references: !self.0.check_footnote_references,
            });
            log::trace!("{plain:?}");
            let txt = plain.as_str();
            let context = SuggestionContext {
                plain: &plain,
                chunk,
                origin,
                allow_concatenated: self.allow_concatenated,
                allow_dashed: self.allow_dashed,
                allow_emojis: self.allow_emojis,
            };

            'tokenization: for range in
                compound_ranges(&self.transform_regex, txt, apply_tokenizer(&self.1, txt))
            {
                let word = sub_chars(txt, range.clone());
                if range.len() == 1
                    && word
                        .chars()
                        .next()
                        .filter(|c| self.ignorelist.contains(*c))
                        .is_some()
                {
                    continue 'tokenization;
                }
                if self.transform_regex.is_empty() {
                    obtain_suggestions(&context, &self.zspell, word, range, &mut acc)
                } else {
                    match transform(&self.transform_regex[..], word.as_str(), range.clone()) {
                        Transformed::Fragments(word_fragments) => {
                            for (range, word_fragment) in word_fragments {
                                obtain_suggestions(
                                    &context,
                                    &self.zspell,
                                    word_fragment.to_owned(),
                                    range,
                                    &mut acc,
                                );
                            }
                        }
                        Transformed::Atomic((range, word)) => {
                            obtain_suggestions(
                                &context,
                                &self.zspell,
                                word.to_owned(),
                                range,
                                &mut acc,
                            );
                        }
                        Transformed::Whitelisted(_) => {}
                    }
                }
            }
        }
        Ok(acc)
    }
}

#[allow(clippy::too_many_arguments)]
fn obtain_suggestions<'s>(
    context: &SuggestionContext<'_, 's>,
    zspell: &zspell::Dictionary,
    word: String,
    range: Range,
    acc: &mut Vec<Suggestion<'s>>,
) {
    let SuggestionContext {
        plain,
        chunk,
        origin,
        allow_concatenated,
        allow_dashed,
        allow_emojis,
    } = *context;
    log::trace!("Checking {word} in {range:?}..");

    match zspell.check_word(&word) {
        false => {
            log::trace!("No match for word (plain range: {range:?}): >{word}<");
            // get rid of single character suggestions
            let replacements = vec![];
            // single char suggestions tend to be useless

            log::debug!(target: "zspell", "{word} --{{suggest}}--> {replacements:?}");

            // strings made of vulgar fraction or emoji
            if allow_emojis && consists_of_vulgar_fractions_or_emojis(&word) {
                log::trace!(target: "quirks", "Found emoji or vulgar fraction character, treating {word} as ok");
                return;
            }

            if allow_concatenated && replacements_contain_dashless(&word, replacements.as_slice()) {
                log::trace!(target: "quirks", "Found dashless word in replacement suggestions, treating {word} as ok");
                return;
            }
            if allow_dashed && replacements_contain_dashed(&word, replacements.as_slice()) {
                log::trace!(target: "quirks", "Found dashed word in replacement suggestions, treating {word} as ok");
                return;
            }
            for (range, span) in plain.find_spans(range.clone()) {
                acc.push(Suggestion {
                    detector: Detector::ZSpell,
                    checked_word: Some(word.clone()),
                    range,
                    span,
                    origin: origin.clone(),
                    replacements: replacements.clone(),
                    chunk,
                    description: Some("Possible spelling mistake found.".to_owned()),
                })
            }
        }
        true => {
            log::trace!("Found a match for word (plain range: {range:?}): >{word}<",);
        }
    }
}
