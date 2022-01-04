//! A set of quirks, not necessarily specific to a checker

use crate::Range;
use fancy_regex::Regex;
use log::{trace, warn};

/// Returns `true` iff the replacements contains a variant of `word` without
/// dashes.
pub(crate) fn replacements_contain_dashless<T: AsRef<str>>(word: &str, replacements: &[T]) -> bool {
    let dashless = word.chars().filter(|c| *c != '-').collect::<String>();
    // if the word does not contain any dashes, skip the replacement iterations
    if dashless == word {
        return false;
    }
    replacements
        .iter()
        .map(|s| s.as_ref())
        .find(|x| *x == &dashless)
        .is_some()
}

/// Returns `true` iff the replacements contains a variant of `word` with
/// additional dashes.
pub(crate) fn replacements_contain_dashed<T: AsRef<str>>(word: &str, replacements: &[T]) -> bool {
    // before doing lots of work, check if the word itself contains a dash, if so
    // the below logic cannot yield and positive results
    if word.chars().find(|c| *c == '-').is_some() {
        return false;
    }

    replacements
        .iter()
        .map(|s| s.as_ref())
        // avoid lots of string iterations in find
        .filter(|s| s.as_bytes().get(0usize) == word.as_bytes().get(0usize))
        .find(|s| itertools::equal(s.chars().filter(|c| *c != '-'), word.chars()))
        .is_some()
}

/// Transformed word with information on the transformation outcome.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Transformed<'i> {
    /// A allow-listed chunk
    Whitelisted((Range, &'i str)),
    /// A set of word-fragments to be checked.
    Fragments(Vec<(Range, &'i str)>),
    /// A word to be checked. Equiv to no match.
    Atomic((Range, &'i str)),
}

/// Transforms a word into a set of fragment-ranges and associated str slices.
pub(crate) fn transform<'i, R: AsRef<Regex>>(
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
            // we try to recursively match the fragments with the regex expr until they become atomic words or whitelisted
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
                        trace!(target:"quirks",
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
                warn!(target:"quirks", "Matching regex >{}< errored: {}", regex.as_str(), e);
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
    use crate::config::WrappedRegex;
    use env_logger;

    #[test]
    fn dashed() {
        let _ = env_logger::builder()
            .is_test(true)
            .filter(None, log::LevelFilter::Trace)
            .try_init();

        const REPLACEMENTS: &'static [&'static str] = &["fffff", "qqq", "z", "zeta-ray"];
        const WORD: &str = "zetaray";
        assert!(replacements_contain_dashed(WORD, REPLACEMENTS));
    }

    #[test]
    fn dashless() {
        let _ = env_logger::builder()
            .is_test(true)
            .filter(None, log::LevelFilter::Trace)
            .try_init();

        const WORD: &str = "zeta-ray";
        const REPLACEMENTS: &'static [&'static str] = &["fffff", "qqq", "z", "zetaray"];
        assert!(replacements_contain_dashless(WORD, REPLACEMENTS));
    }

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
        assert_eq!(
            transform(re.as_slice(), words[0], 10..24),
            Transformed::Whitelisted((10..24, words[0]))
        );

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
