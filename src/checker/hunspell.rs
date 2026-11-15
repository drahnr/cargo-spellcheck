//! A dictionary check with affixes, backed by `libhunspell`
//!
//! Does not check grammar, but tokenizes the documentation chunk, and checks
//! the individual tokens against the dictionary using the defined affixes. Can
//! handle multiple dictionaries.
//!
//! Rust binding panics become errors; failed handles are not reused. This does
//! not contain native faults or builds configured with `panic = "abort"`.

use super::{apply_tokenizer, Checker, Detector, Suggestion, SuggestionContext};

use crate::{
    checker::{cached::cache_builtin, dictaffix::is_valid_hunspell_dic_path},
    config::{Lang5, WrappedRegex},
    documentation::{CheckableChunk, ContentOrigin},
    util::sub_chars,
    Range,
};

use nlprule::Tokenizer;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use hunspell_rs::{CheckResult, Hunspell};

use doc_chunks::Ignores;

#[cfg(test)]
mod tests;

use crate::errors::*;

use super::quirks::{
    compound_ranges, consists_of_vulgar_fractions_or_emojis, replacements_contain_dashed,
    replacements_contain_dashless, transform, Transformed,
};

fn catch_hunspell_panic<T>(operation: &str, action: impl FnOnce() -> Result<T>) -> Result<T> {
    catch_unwind(AssertUnwindSafe(action)).unwrap_or_else(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("non-string panic payload");
        bail!("Hunspell panicked while {operation}: {message}")
    })
}

// Locking assumptions, based on bundled Hunspell in hunspell-sys 0.3.1:
// - Each engine owns its AffixMgr. Lookups mutate fields such as pfx/sfx, so
//   calls on the same engine require its exclusive mutex, not RwLock readers.
// - Construction, dictionary addition, and destruction update the shared UTF-8
//   table's non-atomic reference count and require this process-wide lock.
//   With balanced references, live UTF-8 engines pin the read-only table, so
//   lookups on distinct engines do not need the lifecycle lock.
// - Bundled AffixMgr incorrectly releases a reference for non-UTF-8 engines.
//   The builtin SET UTF-8 header avoids that trigger; mixed-encoding engine
//   lifetimes remain affected by the native bug, even with serialized calls.
// This relies on native internals, not the Rust binding's &self signatures.
// Re-audit when upgrading/replacing the native library. Native handles created
// outside this wrapper are not coordinated by these locks.
static HUNSPELL_LIFECYCLE: Mutex<()> = Mutex::new(());

struct NativeHunspell(Option<Hunspell>);

// SAFETY: We assume native handles have no thread affinity. HunspellSafe keeps
// this owner behind an exclusive mutex; the lifecycle lock protects shared
// native state as described above.
unsafe impl Send for NativeHunspell {}

impl Drop for NativeHunspell {
    fn drop(&mut self) {
        let _lifecycle = HUNSPELL_LIFECYCLE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Err(error) = catch_hunspell_panic("destroying the native dictionary", || {
            drop(self.0.take());
            Ok(())
        }) {
            log::error!("{error}");
        }
    }
}

#[derive(Clone)]
struct HunspellSafe(Arc<Mutex<NativeHunspell>>);

impl HunspellSafe {
    fn new(aff: &str, dic: &str) -> Result<Self> {
        let engine = {
            let _lifecycle = HUNSPELL_LIFECYCLE
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            catch_hunspell_panic("initializing the native dictionary", || {
                Ok(Hunspell::new(aff, dic))
            })?
        };
        Ok(Self(Arc::new(Mutex::new(NativeHunspell(Some(engine))))))
    }

    fn run_action<T>(&self, operation: &str, action: impl FnOnce(&mut Hunspell) -> T) -> Result<T> {
        catch_hunspell_panic(operation, || {
            // Lock inside the unwind boundary so a panicking call poisons the
            // shared handle. Never reuse potentially inconsistent native state.
            let mut engine = self.0.lock().map_err(|_| {
                eyre!("Hunspell is unavailable after an earlier panic; recreate the checker before {operation}")
            })?;
            let engine = engine
                .0
                .as_mut()
                .ok_or_else(|| eyre!("Hunspell dictionary has been destroyed"))?;
            Ok(action(engine))
        })
    }

    fn check(&self, word: &str) -> Result<CheckResult> {
        self.run_action(&format!("checking word {word:?}"), |engine| {
            engine.check(word)
        })
    }

    fn suggest(&self, word: &str) -> Result<Vec<String>> {
        self.run_action(&format!("suggesting replacements for {word:?}"), |engine| {
            engine.suggest(word)
        })
    }

    fn add_dictionary(&self, path: &str) -> Result<bool> {
        self.run_action(&format!("adding dictionary {path:?}"), |engine| {
            let _lifecycle = HUNSPELL_LIFECYCLE
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            engine.add_dictionary(path)
        })
    }
}

#[derive(Clone)]
pub struct HunspellCheckerInner {
    hunspell: HunspellSafe,
    transform_regex: Vec<WrappedRegex>,
    allow_concatenated: bool,
    allow_dashed: bool,
    allow_emojis: bool,
    check_footnote_references: bool,
    ignorelist: String,
}

impl HunspellCheckerInner {
    fn new(config: &<HunspellChecker as Checker>::Config) -> Result<Self> {
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

        // setup hunspell:
        let search_dirs = config.search_dirs();

        let lang = config.lang().to_string();
        let lang = lang.as_str();

        // lookup paths are really just an attempt to provide a dictionary, so be more forgiving
        // when encountering errors here
        let (dic, aff): (PathBuf, PathBuf) = search_dirs
            .into_iter()
            .filter(|search_dir| {
                let keep = search_dir.is_dir();
                if !keep {
                    // search_dir also contains the default paths, so just silently ignore these
                    log::debug!(
                        "Dictionary search path is not a directory {}",
                        search_dir.display()
                    );
                } else {
                    log::debug!(
                        "Found dictionary search path {}",
                        search_dir.display()
                    );
                }
                keep
            })
            .find_map(|search_dir| {
                let dic = search_dir.join(lang).with_extension("dic");
                if !dic.is_file() {
                    log::debug!(
                        "Dictionary path dervied from search dir is not a file {}",
                        dic.display()
                    );
                    return None;
                }
                let aff = search_dir.join(lang).with_extension("aff");
                if !aff.is_file() {
                    log::debug!(
                        "Affixes path dervied from search dir is not a file {}",
                        aff.display()
                    );
                    return None;
                }
                log::debug!("Using dic {} and aff {}", dic.display(), aff.display());
                Some((dic, aff))
            })
            .ok_or_else(|| {
                eyre!("Failed to find any {lang}.dic / {lang}.aff in any search dir or no search provided",
                    lang = lang)
            })
            .or_else(|e| {
                if config.use_builtin {
                    Ok(cache_builtin()?)
                } else {
                    Err(e)
                }
            })?;

        let dic = dic.to_str().unwrap();
        let aff = aff.to_str().unwrap();

        let hunspell = HunspellSafe::new(aff, dic)?;
        is_valid_hunspell_dic_path(dic)?;
        hunspell.add_dictionary(dic)?;

        if cfg!(debug_assertions) && Lang5::en_US == lang {
            // "Test" is a valid word
            debug_assert_eq!(hunspell.check("Test")?, CheckResult::FoundInDictionary);
            // suggestion must contain the word itself if it is valid
            debug_assert!(hunspell.suggest("Test")?.contains(&"Test".to_string()));
        }

        // suggestion must contain the word itself if it is valid extra dictionary
        // be more strict about the extra dictionaries, they have to exist
        for extra_dic in config.extra_dictionaries() {
            log::debug!("Adding extra dictionary {}", extra_dic.display());
            if !extra_dic.is_file() {
                bail!("Extra dictionary {} is not a file", extra_dic.display());
            }
            is_valid_hunspell_dic_path(extra_dic)?;
            if let Some(extra_dic) = extra_dic.to_str() {
                if !hunspell.add_dictionary(extra_dic)? {
                    bail!("Failed to add extra dictionary path to context {extra_dic}");
                }
            } else {
                bail!(
                    "Failed to convert extra dictionary path to str {}",
                    extra_dic.display()
                );
            }
        }
        log::debug!("Dictionary setup completed successfully.");
        Ok(Self {
            hunspell,
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
pub struct HunspellChecker(pub Arc<HunspellCheckerInner>, pub Arc<Tokenizer>);

impl std::ops::Deref for HunspellChecker {
    type Target = HunspellCheckerInner;
    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl HunspellChecker {
    pub fn new(config: &<HunspellChecker as Checker>::Config) -> Result<Self> {
        let tokenizer = super::tokenizer::<&PathBuf>(None)?;
        let inner = catch_hunspell_panic("initializing the checker", || {
            HunspellCheckerInner::new(config)
        })?;
        let hunspell = Arc::new(inner);
        Ok(HunspellChecker(hunspell, tokenizer))
    }
}

impl Checker for HunspellChecker {
    type Config = crate::config::HunspellConfig;

    fn detector() -> Detector {
        Detector::Hunspell
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
            let hunspell = &self.hunspell;
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
                    obtain_suggestions(&context, hunspell, word, range, &mut acc)?;
                } else {
                    match transform(&self.transform_regex[..], word.as_str(), range.clone()) {
                        Transformed::Fragments(word_fragments) => {
                            for (range, word_fragment) in word_fragments {
                                obtain_suggestions(
                                    &context,
                                    hunspell,
                                    word_fragment.to_owned(),
                                    range,
                                    &mut acc,
                                )?;
                            }
                        }
                        Transformed::Atomic((range, word)) => {
                            obtain_suggestions(
                                &context,
                                hunspell,
                                word.to_owned(),
                                range,
                                &mut acc,
                            )?;
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
    hunspell: &HunspellSafe,
    word: String,
    range: Range,
    acc: &mut Vec<Suggestion<'s>>,
) -> Result<()> {
    let SuggestionContext {
        plain,
        chunk,
        origin,
        allow_concatenated,
        allow_dashed,
        allow_emojis,
    } = *context;
    log::trace!("Checking {word} in {range:?}..");

    match hunspell.check(&word)? {
        CheckResult::MissingInDictionary => {
            log::trace!("No match for word (plain range: {range:?}): >{word}<");
            // get rid of single character suggestions
            let replacements =
                Vec::from_iter(hunspell.suggest(&word)?.into_iter().filter(|x| x.len() > 1));
            // single char suggestions tend to be useless

            log::debug!(target: "hunspell", "{word} --{{suggest}}--> {replacements:?}");

            // strings made of vulgar fraction or emoji
            if allow_emojis && consists_of_vulgar_fractions_or_emojis(&word) {
                log::trace!(target: "quirks", "Found emoji or vulgar fraction character, treating {word} as ok");
                return Ok(());
            }

            if allow_concatenated && replacements_contain_dashless(&word, replacements.as_slice()) {
                log::trace!(target: "quirks", "Found dashless word in replacement suggestions, treating {word} as ok");
                return Ok(());
            }
            if allow_dashed && replacements_contain_dashed(&word, replacements.as_slice()) {
                log::trace!(target: "quirks", "Found dashed word in replacement suggestions, treating {word} as ok");
                return Ok(());
            }
            for (range, span) in plain.find_spans(range.clone()) {
                acc.push(Suggestion {
                    detector: Detector::Hunspell,
                    range,
                    span,
                    origin: origin.clone(),
                    replacements: replacements.clone(),
                    chunk,
                    description: Some("Possible spelling mistake found.".to_owned()),
                })
            }
        }
        CheckResult::FoundInDictionary => {
            log::trace!("Found a match for word (plain range: {range:?}): >{word}<",);
        }
    }
    Ok(())
}
