//! A dictionary check with affixes, backed by `libhunspell`
//!
//! Does not check grammar, but tokenizes the documentation chunk,
//! and checks the individual tokens against the dictionary using
//! the defined affixes.
//! Can handle multiple dictionaries.

use super::{tokenize, Checker, Detector, Documentation, Suggestion, SuggestionSet};
use std::path::PathBuf;

use log::{debug, trace};

use hunspell_rs::Hunspell;

use anyhow::{anyhow, Result};

pub struct HunspellChecker;

impl Checker for HunspellChecker {
    type Config = crate::config::HunspellConfig;
    fn check<'a, 's>(docu: &'a Documentation, config: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's,
    {
        // let hunspell = lazy_static::lazy_static!{
        //     static ref HUNSPELL_CTX: Result<Hunspell> = {

        //     }
        // };

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
                        "Dictionary search path {} is not a directory",
                        search_dir.display()
                    );
                }
                keep
            })
            .find_map(|search_dir| {
                let dic = search_dir.join(lang).with_extension("dic");
                if !dic.is_file() {
                    debug!(
                        "Dictionary path dervied from search dir {} is not a file",
                        dic.display()
                    );
                    return None;
                }
                let aff = search_dir.join(lang).with_extension("aff");
                if !aff.is_file() {
                    debug!(
                        "Affixes path dervied from search dir {} is not a file",
                        aff.display()
                    );
                    return None;
                }
                trace!("Using dic {} and aff {}", dic.display(), aff.display());
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
            assert!(hunspell.check("Test"));
            // suggestion must contain the word itself if it is valid
            assert!(hunspell.suggest("Test").contains(&"Test".to_string()));
        }

        // be more strict about the extra dictionaries, they have to exist
        for extra_dic in config.extra_dictonaries().iter() {
            trace!("Adding extra hunspell dictionary {}", extra_dic.display());
            if !extra_dic.is_file() {
                return Err(anyhow!(
                    "Extra dictionary {} is not a file",
                    extra_dic.display()
                ));
            }
            if let Some(extra_dic) = extra_dic.to_str() {
                if !hunspell.add_dictionary(extra_dic) {
                    return Err(anyhow!("Failed to add additional dict to hunspell"));
                }
            } else {
                return Err(anyhow!(
                    "Failed to convert one of the extra dictionaries to a str"
                ));
            }
        }

        let suggestions = docu.iter().try_fold::<SuggestionSet, _, Result<_>>(
            SuggestionSet::new(),
            |mut acc, (origin, chunks)| {
                for chunk in chunks {
                    let plain = chunk.erase_markdown();
                    trace!("{:?}", &plain);
                    let txt = plain.as_str();
                    for range in tokenize(txt) {
                        let word = &txt[range.clone()];
                        if !hunspell.check(word) {
                            trace!("No match for word (plain range: {:?}): >{}<", &range, word);
                            // get rid of single character suggestions
                            let replacements = hunspell
                                .suggest(word)
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
                                        description: Some(
                                            "Possible spelling mistake found.".to_owned(),
                                        ),
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
                }
                Ok(acc)
            },
        )?;

        // TODO sort spans by file and line + column
        Ok(suggestions)
    }
}
