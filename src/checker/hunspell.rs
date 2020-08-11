//! A dictionary check with affixes, backed by `libhunspell`
//!
//! Does not check grammar, but tokenizes the documentation chunk,
//! and checks the individual tokens against the dictionary using
//! the defined affixes.
//! Can handle multiple dictionaries.

use super::{tokenize, Checker, Detector, Documentation, Suggestion, SuggestionSet};
use crate::documentation::CheckableChunk;
use crate::documentation::ContentOrigin;
use crate::documentation::PlainOverlay;
use crate::util::sub_chars;
use crate::Range;
use log::{debug, trace, warn};
use std::collections::VecDeque;
use std::path::PathBuf;

use hunspell_rs::Hunspell;

use anyhow::{anyhow, bail, Result};

pub struct HunspellChecker;

impl HunspellChecker {
    fn inner_init(config: &<Self as Checker>::Config) -> Result<Hunspell> {
        let search_dirs = config.search_dirs();

        let lang = config.lang();

        // lookup paths are really just an attempt to provide a dictionary, so be more forgiving
        // when encountering errors here
        let (dic, aff): (PathBuf, PathBuf) = search_dirs
            .into_iter()
            .filter(|search_dir| {
                let keep = search_dir.is_dir();
                if !keep {
                    // search_dir also contains the default paths, so just silently ignore these
                    debug!(
                        "Dictionary search path is not a directory {}",
                        search_dir.display()
                    );
                } else {
                    debug!(
                        "Found dictionary search path {}",
                        search_dir.display()
                    );
                }
                keep
            })
            .find_map(|search_dir| {
                let dic = search_dir.join(lang).with_extension("dic");
                if !dic.is_file() {
                    debug!(
                        "Dictionary path dervied from search dir is not a file {}",
                        dic.display()
                    );
                    return None;
                }
                let aff = search_dir.join(lang).with_extension("aff");
                if !aff.is_file() {
                    debug!(
                        "Affixes path dervied from search dir is not a file {}",
                        aff.display()
                    );
                    return None;
                }
                debug!("Using dic {} and aff {}", dic.display(), aff.display());
                Some((dic, aff))
            })
            .ok_or_else(|| {
                anyhow!("Failed to find any {lang}.dic / {lang}.aff in any search dir or no search provided",
                lang = lang)
            })?;

        let dic = dic.to_str().unwrap();
        let aff = aff.to_str().unwrap();

        let mut hunspell = Hunspell::new(aff, dic);
        hunspell.add_dictionary(dic);

        if cfg!(debug_assertions) && lang == "en_US" {
            // "Test" is a valid word
            debug_assert!(hunspell.check("Test"));
            // suggestion must contain the word itself if it is valid
            debug_assert!(hunspell.suggest("Test").contains(&"Test".to_string()));
        }

        // suggestion must contain the word itself if it is valid extra dictionary
        // be more strict about the extra dictionaries, they have to exist
        for extra_dic in config.extra_dictonaries().iter() {
            trace!("Adding extra dictionary {}", extra_dic.display());
            if !extra_dic.is_file() {
                bail!("Extra dictionary {} is not a file", extra_dic.display())
            }
            if let Some(extra_dic) = extra_dic.to_str() {
                if !hunspell.add_dictionary(extra_dic) {
                    bail!(
                        "Failed to add extra dictionary path to context {}",
                        extra_dic
                    )
                }
            } else {
                bail!(
                    "Failed to convert extra dictionary path to str {}",
                    extra_dic.display()
                )
            }
        }
        Ok(hunspell)
    }
}

impl Checker for HunspellChecker {
    type Config = crate::config::HunspellConfig;
    fn check<'a, 's>(docu: &'a Documentation, config: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's,
    {
        let hunspell = Self::inner_init(config)?;

        let transform_regex = config.transform_regex.clone().unwrap_or_else(|| Vec::new());

        let suggestions = docu.iter().try_fold::<SuggestionSet, _, Result<_>>(
            SuggestionSet::new(),
            move |mut acc, (origin, chunks)| {
                debug!("Processing {}", origin.as_path().display());



                for chunk in chunks {
                    let plain = chunk.erase_markdown();
                    trace!("{:?}", &plain);
                    let txt = plain.as_str();
                    for range in tokenize(txt) {
                        let word = sub_chars(txt, range.clone());
                        if transform_regex.is_empty() {
                            obtain_suggestions(&plain, chunk, &hunspell, origin, word, range, &mut acc)
                        } else {
                            let mut matched = false;
                            for regex in transform_regex.iter() {
                                match regex.captures(word.as_str()) {
                                    Ok(Some(captures)) => {
                                        matched = true;
                                        // first one is always the full match
                                        if captures.len() == 1 {
                                            // means match, but no captures,
                                            // which is equiv to an implicit whitelist
                                            break;
                                        }
                                        for m in captures.iter().skip(1).filter_map(|x| x) {
                                            trace!("found capture for word >{}<, with match >{}< and capture >{}<", captures.get(0).unwrap().as_str(), word.as_str(), m.as_str());
                                            let offset = word.char_indices().take_while(|(offset, _)| m.start() < *offset).count();
                                            let range = Range {
                                                start: range.start + offset,
                                                end: range.start + offset + m.as_str().chars().count(),
                                            };
                                            obtain_suggestions(&plain, chunk, &hunspell, origin, m.as_str().to_owned(), range, &mut acc);
                                        }
                                        break;
                                    },
                                    Ok(None) => {
                                        // no regex match, try the next regex
                                        continue;
                                    },
                                    Err(e) => {
                                        warn!("Matching regex >{}< errored: {}", regex.as_str(), e);
                                        break;
                                    }
                                }
                            }
                            // nothing matched, check the entire word
                            if !matched {
                                obtain_suggestions(&plain, chunk, &hunspell, origin, word, range, &mut acc)
                            }
                        }
                    }
                }
                Ok(acc)
            },
        )?;

        // TODO sort spans by file and line + column
        Ok(suggestions)
    }
}

fn obtain_suggestions<'s>(
    plain: &PlainOverlay,
    chunk: &'s CheckableChunk,
    hunspell: &Hunspell,
    origin: &ContentOrigin,
    word: String,
    range: Range,
    acc: &mut SuggestionSet<'s>,
) {
    if !hunspell.check(&word) {
        trace!("No match for word (plain range: {:?}): >{}<", &range, &word);
        // get rid of single character suggestions
        let replacements = hunspell
            .suggest(&word)
            .into_iter()
            .filter(|x| x.len() > 1) // single char suggestions tend to be useless
            .collect::<Vec<_>>();

        for (range, span) in plain.find_spans(range.clone()) {
            acc.add(
                origin.clone(),
                Suggestion {
                    detector: Detector::Hunspell,
                    range,
                    span,
                    origin: origin.clone(),
                    replacements: replacements.clone(),
                    chunk,
                    description: Some("Possible spelling mistake found.".to_owned()),
                },
            )
        }
    } else {
        trace!(
            "Found a match for word (plain range: {:?}): >{}<",
            &range,
            word
        );
    }
}
