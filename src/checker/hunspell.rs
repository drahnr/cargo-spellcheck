//! A dictionary check with affixes, backed by `libhunspell`
//!
//! Does not check grammar, but tokenizes the documentation chunk,
//! and checks the individual tokens against the dictionary using
//! the defined affixes.
//! Can handle multiple dictionaries.

use super::{apply_tokenizer, Checker, Detector, Documentation, Suggestion, SuggestionSet};

use crate::documentation::{CheckableChunk, ContentOrigin, PlainOverlay};
use crate::util::sub_chars;
use crate::Range;

use fs_err as fs;
use log::{debug, trace};
use rayon::prelude::*;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hunspell_rs::Hunspell;

use anyhow::{anyhow, bail, Result};

use super::quirks::{
    replacements_contain_dashed, replacements_contain_dashless, transform, Transformed,
};

pub struct HunspellWrapper(pub Arc<Hunspell>);

unsafe impl Send for HunspellWrapper {}
unsafe impl Sync for HunspellWrapper {}

pub struct HunspellChecker;

impl HunspellChecker {
    fn inner_init(config: &<Self as Checker>::Config) -> Result<HunspellWrapper> {
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
        is_valid_hunspell_dic_path(dic)?;
        hunspell.add_dictionary(dic);

        if cfg!(debug_assertions) && lang == "en_US" {
            // "Test" is a valid word
            debug_assert!(hunspell.check("Test"));
            // suggestion must contain the word itself if it is valid
            debug_assert!(hunspell.suggest("Test").contains(&"Test".to_string()));
        }

        // suggestion must contain the word itself if it is valid extra dictionary
        // be more strict about the extra dictionaries, they have to exist
        for extra_dic in config.extra_dictionaries().iter() {
            debug!("Adding extra dictionary {}", extra_dic.display());
            if !extra_dic.is_file() {
                bail!("Extra dictionary {} is not a file", extra_dic.display())
            }
            is_valid_hunspell_dic_path(extra_dic)?;
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
        debug!("Dictionary setup completed successfully.");
        Ok(HunspellWrapper(Arc::new(hunspell)))
    }
}

impl Checker for HunspellChecker {
    type Config = crate::config::HunspellConfig;

    fn detector() -> Detector {
        Detector::Hunspell
    }

    fn check<'a, 's>(docu: &'a Documentation, config: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's,
    {
        let hunspell = Self::inner_init(config)?;

        let (transform_regex, allow_concatenated, allow_dashed) = {
            let quirks = &config.quirks;
            {
                (
                    quirks.transform_regex(),
                    quirks.allow_concatenated(),
                    quirks.allow_dashed(),
                )
            }
        };
        // FIXME rename the config option
        let ignorelist = config.tokenization_splitchars.as_str();
        // without these, a lot of those would be flagged as mistakes.
        debug_assert!(ignorelist.contains(','));
        debug_assert!(ignorelist.contains('.'));
        debug_assert!(ignorelist.contains(';'));
        debug_assert!(ignorelist.contains('!'));
        debug_assert!(ignorelist.contains('?'));

        // TODO allow override
        let tokenizer = super::tokenizer::<&PathBuf>(None)?;

        let suggestions = docu
            .par_iter()
            .try_fold::<SuggestionSet, Result<_>, _, _>(
                || SuggestionSet::new(),
                move |mut acc, (origin, chunks)| {
                    debug!("Processing {}", origin.as_path().display());

                    for chunk in chunks {
                        let plain = chunk.erase_cmark();
                        trace!("{:?}", &plain);
                        let txt = plain.as_str();
                        let hunspell = &*hunspell.0;

                        'tokenization: for range in apply_tokenizer(&tokenizer, txt) {
                            let word = sub_chars(txt, range.clone());
                            if range.len() == 1 && word.chars().next().filter(|c| ignorelist.contains(*c)).is_some() {
                                continue 'tokenization;
                            }
                            if transform_regex.is_empty() {
                                obtain_suggestions(
                                    &plain,
                                    chunk,
                                    &hunspell,
                                    origin,
                                    word,
                                    range,
                                    allow_concatenated,
                                    allow_dashed,
                                    &mut acc,
                                )
                            } else {
                                match transform(&transform_regex[..], word.as_str(), range.clone())
                                {
                                    Transformed::Fragments(word_fragments) => {
                                        for (range, word_fragment) in word_fragments {
                                            obtain_suggestions(
                                                &plain,
                                                chunk,
                                                &hunspell,
                                                origin,
                                                word_fragment.to_owned(),
                                                range,
                                                allow_concatenated,
                                                allow_dashed,
                                                &mut acc,
                                            );
                                        }
                                    }
                                    Transformed::Atomic((range, word)) => {
                                        obtain_suggestions(
                                            &plain,
                                            chunk,
                                            &hunspell,
                                            origin,
                                            word.to_owned(),
                                            range,
                                            allow_concatenated,
                                            allow_dashed,
                                            &mut acc,
                                        );
                                    }
                                    Transformed::Whitelisted(_) => {}
                                }
                            }
                        }
                    }
                    Ok(acc)
                },
            )
            .try_reduce(
                || SuggestionSet::new(),
                |mut a, b| {
                    a.join(b);
                    Ok(a)
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
    allow_concatenated: bool,
    allow_dashed: bool,
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

        if allow_concatenated && replacements_contain_dashless(&word, replacements.as_slice()) {
            trace!(target: "quirks", "Found dashless word in replacement suggestions, treating {} as ok", &word);
            return;
        }
        if allow_dashed && replacements_contain_dashed(&word, replacements.as_slice()) {
            trace!(target: "quirks", "Found dashed word in replacement suggestions, treating {} as ok", &word);
            return;
        }
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

/// Check if provided path has valid dictionary format.
///
/// This is a YOLO check.
fn is_valid_hunspell_dic_path(path: impl AsRef<Path>) -> Result<()> {
    let reader = io::BufReader::new(fs::File::open(path.as_ref())?);
    is_valid_hunspell_dic(reader)
}

/// Check a reader for correct hunspell format.
fn is_valid_hunspell_dic(reader: impl BufRead) -> Result<()> {
    let mut iter = reader.lines().enumerate();
    if let Some((_lineno, first)) = iter.next() {
        let first = first?;
        let _ = first.parse::<u64>().map_err(|e| {
            anyhow!(
                "First line of extra dictionary must a number, but is: >{}<",
                first
            )
            .context(e)
        })?;
    }
    // Just check the first 10 lines, don't waste much time here
    // the first two are the most important ones.
    for (lineno, line) in iter.take(10) {
        // All lines after must be format x.
        if let Ok(num) = line?.parse::<i64>() {
            bail!(
                "Line {} of extra dictionary must not be a number, but is: >{}<",
                lineno,
                num
            )
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunspell_dic_format() {
        const GOOD: &str = "2
whitespazes
catsndogs
";
        const BAD_1: &str = "foo
12349
bar
";
        const BAD_2: &str = "2
12349
bar
";
        const BAD_3: &str = "foo
xxx
bar
";
        assert!(is_valid_hunspell_dic(&mut GOOD.as_bytes()).is_ok());
        assert!(is_valid_hunspell_dic(&mut BAD_1.as_bytes()).is_err());
        assert!(is_valid_hunspell_dic(&mut BAD_2.as_bytes()).is_err());
        assert!(is_valid_hunspell_dic(&mut BAD_3.as_bytes()).is_err());
    }

    #[test]
    fn hunspell_binding_is_sane() {
        let config = crate::config::HunspellConfig::default();
        let search_dirs = config.search_dirs();

        let mut srcs = None;
        for search_dir in search_dirs {
            let dic = search_dir.join("en_US.dic");
            let aff = search_dir.join("en_US.aff");
            if dic.is_file() && aff.is_file() && is_valid_hunspell_dic_path(&dic).is_ok() {
                srcs = Some((dic, aff));
                break;
            }
        }

        let (dic, aff) = srcs.unwrap();

        let mut hunspell = Hunspell::new(
            aff.display().to_string().as_str(),
            dic.display().to_string().as_str(),
        );
        let cwd = crate::traverse::cwd().unwrap();
        let extra = dbg!(cwd.join(".config/lingo.dic"));
        assert!(extra.is_file());
        assert!(is_valid_hunspell_dic_path(&dic).is_ok());

        hunspell.add_dictionary(dbg!(extra.display().to_string().as_str()));

        let extra_dic = io::BufReader::new(fs::File::open(extra).unwrap());
        for (lineno, line) in extra_dic.lines().enumerate().skip(1) {
            let line = line.unwrap();
            let word = if line.contains('/') {
                line.split('/').next().unwrap()
            } else {
                line.as_str()
            };

            println!("testing >{}< against line #{} >{}<", word, lineno, line);
            // "whitespace" is a word part of our custom dictionary
            assert!(hunspell.check(word));
            // Technically suggestion must contain the word itself if it is valid
            let suggestions = hunspell.suggest(word);
            // but this is not true for i.e. `clang`
            // assert!(suggestions.contains(&word.to_owned()));
            if !suggestions.contains(&word.to_owned()) {
                eprintln!(
                    "suggest does not contain valid self: {} ∉ {:?}",
                    word, suggestions
                );
            }
        }
    }
}
