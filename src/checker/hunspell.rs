//! A dictionary check with affixes, backed by `libhunspell`
//!
//! Does not check grammar, but tokenizes the documentation chunk,
//! and checks the individual tokens against the dictionary using
//! the defined affixes.
//! Can handle multiple dictionaries.

use super::{tokenize, Checker, Detector, Documentation, Suggestion, SuggestionSet};
use crate::config::WrappedRegex;
use crate::documentation::{CheckableChunk, ContentOrigin, PlainOverlay};
use crate::util::sub_chars;
use crate::Range;
use fancy_regex::Regex;
use log::{debug, trace, warn};
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
                            obtain_suggestions(
                                &plain, chunk, &hunspell, origin, word, range, &mut acc,
                            )
                        } else {
                            match transform(&transform_regex[..], word.as_str(), range.clone()) {
                                Transformed::Fragments(word_fragments) => {
                                    for (range, word_fragment) in word_fragments {
                                        obtain_suggestions(
                                            &plain,
                                            chunk,
                                            &hunspell,
                                            origin,
                                            word_fragment.to_owned(),
                                            range,
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
                                        &mut acc,
                                    );
                                }
                                Transformed::Whitelisted(_) => {},
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

/// Transformed word with information on the transformation outcome.
#[derive(Debug, Eq, PartialEq)]
enum Transformed<'i> {
    /// A whitelisted chunk
    Whitelisted((Range, &'i str)),
    /// A set of word-fragments to be checked.
    Fragments(Vec<(Range, &'i str)>),
    /// A word to be checked. Equiv to no match.
    Atomic((Range, &'i str)),
}

/// Transforms a word into a set of fragment-ranges and associated str slices.
fn transform<'i, R: AsRef<Regex>>(
    transform_regex: &[R],
    word: &'i str,
    range: Range,
) -> Transformed<'i> {

    let mut q = std::collections::VecDeque::<(Range, &'_ str)>::with_capacity(32);
    let mut words = Vec::with_capacity(16);
    let mut whitelisted = 0usize;
    q.push_back((range.clone(), word));
    while let Some((range, word)) = q.pop_front() {

        // work on a fragment now
        match transform_inner(transform_regex, word, range.clone()) {
            // we try to match the fragments with the regex expr until they become atomic words or whitelisted
            Transformed::Fragments(v) => q.extend(v),
            Transformed::Atomic(word) => words.push(word),
            Transformed::Whitelisted(_) => whitelisted += 1,
        }
    }

    // no match found at all, this word is "atomic" and will be checked as is
    if whitelisted == 0usize {
        // empty means nothing, one word with the same range means we only found the initial provided word
        if words.is_empty() || (words.len() == 1 && words[0].0.len() == word.len()) {
            return Transformed::Atomic((range, word));
        }
    }

    if !words.is_empty() {
        // collect all the words as fragments again (they actually really are)
        Transformed::Fragments(words)
    } else {
        // if there are no words to be checked, everything is whitelisted
        Transformed::Whitelisted((range, word))
    }
}


/// Inner loop transform
///
/// Returns `Some(vec![..])` if any captures were found.
fn transform_inner<'i, R: AsRef<Regex>>(
    transform_regex: &[R],
    word: &'i str,
    range: Range,
) -> Transformed<'i> {
    for regex in transform_regex.iter().map(AsRef::as_ref) {
        match regex.captures(word) {
            Ok(Some(captures)) => {
                // first one is always the full match
                if captures.len() == 1 {
                    // means match, but no captures,
                    // which is equiv to an implicit whitelist
                    return Transformed::Whitelisted((range, word));
                }
                let intermediate = captures
                    .iter()
                    .skip(1)
                    .filter_map(|m_opt| m_opt)
                    .map(|m| {
                        let intra_word_range = m.start()..m.end();
                        trace!(
                            "Found capture for word >{}<, with match >{}< and capture >{}< at {:?}",
                            captures.get(0).unwrap().as_str(),
                            word,
                            m.as_str(),
                            &intra_word_range
                        );
                        let offset = word
                            .char_indices()
                            .take_while(|(byte_pos, _)| m.start() > *byte_pos)
                            .count();
                        let range = Range {
                            start: range.start + offset,
                            end: range.start + offset + m.as_str().chars().count(),
                        };
                        (range, &word[intra_word_range])
                    })
                    .collect::<Vec<_>>();

                return Transformed::Fragments(intermediate);
            }
            Ok(None) => {
                // no regex match, try the next regex
                continue;
            }
            Err(e) => {
                warn!("Matching regex >{}< errored: {}", regex.as_str(), e);
                break;
            }
        }
    }
    // nothing matched, check the entire word instead
    Transformed::Atomic((range, word))
}

#[cfg(test)]
mod tests {
    use super::*;
    use env_logger;
    #[test]
    fn transformer() {
        let _ = env_logger::builder()
            .is_test(true)
            .filter(None, log::LevelFilter::Trace)
            .try_init();

        let re = vec![
            WrappedRegex::from(Regex::new("^[0-9]+x$").unwrap()), //whitelist
            WrappedRegex::from(Regex::new(r#"^'([^\s]+)'$"#).unwrap()),
            WrappedRegex::from(Regex::new("(Alpha)(beta)").unwrap()),
        ];

        let words = vec!["2x", r#"''so-to-speak''"#, "Alphabeta", "Nothing"];

        // whitelist
        assert_eq!(transform(re.as_slice(), words[0], 10..24), Transformed::Whitelisted((10..24, words[0])));

        // single quoted, recursive 2x
        assert_eq!(
            transform(re.as_slice(), words[1], 10..25),
            Transformed::Fragments(vec![(12..23, &words[1][2..13])])
        );

        // multi capture
        assert_eq!(
            transform(re.as_slice(), words[2], 10..19),
            Transformed::Fragments(vec![(10..15, &words[2][0..5]), (15..19, &words[2][5..9]),])
        );

        // no match
        assert_eq!(
            transform(re.as_slice(), words[3], 10..17),
            Transformed::Atomic((10..17, words[3]))
        );
    }
}
