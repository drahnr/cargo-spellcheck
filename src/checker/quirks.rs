//! A set of quirks, not necessarily specific to a checker

use crate::Range;
use fancy_regex::Regex;

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
        .any(|x| x == dashless)
}

/// Returns `true` iff the replacements contains a variant of `word` with
/// additional dashes.
pub(crate) fn replacements_contain_dashed<T: AsRef<str>>(word: &str, replacements: &[T]) -> bool {
    // before doing lots of work, check if the word itself contains a dash, if so
    // the below logic cannot yield and positive results
    if word.chars().any(|c| c == '-') {
        return false;
    }

    replacements
        .iter()
        .map(|s| s.as_ref())
        // avoid lots of string iterations in find
        .filter(|s| s.as_bytes().first() == word.as_bytes().first())
        .any(|s| itertools::equal(s.chars().filter(|c| *c != '-'), word.chars()))
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

/// Exclude no-capture regex matches that overlap at least two tokens.
///
/// Token ranges must be ordered and non-overlapping, with character (not byte) offsets.
/// Matches within a single token retain the existing per-token transformation policy.
pub(crate) fn allow_list_ranges<R: AsRef<Regex>>(
    transform_regex: &[R],
    text: &str,
    ranges: impl Iterator<Item = Range>,
) -> Vec<Range> {
    let ranges = Vec::from_iter(ranges);
    // Capture groups select fragments for checking; only capture-free regexes allow-list.
    let regexes = Vec::from_iter(
        transform_regex
            .iter()
            .map(AsRef::as_ref)
            .filter(|regex| regex.captures_len() == 1),
    );
    if regexes.is_empty() || ranges.len() < 2 {
        return ranges;
    }
    // Regex matches use byte offsets, while tokenizer ranges use character offsets.
    // Including the final boundary also permits matches ending at the end of the text.
    let char_boundaries = Vec::from_iter(
        text.char_indices()
            .map(|(offset, _)| offset)
            .chain(std::iter::once(text.len())),
    );
    let mut excluded = Vec::new();
    let mut byte_offset = 0;
    // Keep word-oriented anchors and wildcards from swallowing neighboring words.
    for run in text.split_inclusive(char::is_whitespace) {
        let word = run.trim_end_matches(char::is_whitespace);
        for regex in &regexes {
            for matched in regex.find_iter(word) {
                let matched = match matched {
                    Ok(matched) => matched,
                    Err(error) => {
                        log::warn!(target: "quirks", "Matching regex >{}< errored: {}", regex.as_str(), error);
                        break;
                    }
                };
                if matched.start() == matched.end() {
                    continue;
                }
                let start = char_boundaries
                    .binary_search(&(byte_offset + matched.start()))
                    .expect("regex matches start at a character boundary");
                let end = char_boundaries
                    .binary_search(&(byte_offset + matched.end()))
                    .expect("regex matches end at a character boundary");
                let first = ranges.partition_point(|range| range.end <= start);
                // A second overlapping token distinguishes cross-token matches from
                // matches that must still be handled by `transform` per token.
                if ranges.get(first + 1).is_some_and(|range| range.start < end) {
                    excluded.push(start..end);
                }
            }
        }
        byte_offset += run.len();
    }
    subtract_ranges(ranges, excluded)
}

/// Remove allow-listed portions of tokens, returning the fragments still needing checks.
///
/// Both inputs use half-open character offsets into the same source text, not byte offsets:
/// - `ranges` contains the nonempty token ranges, ordered by position and non-overlapping.
/// - `excluded` contains the matched portions to skip. These may overlap, span multiple
///   tokens, or arrive in any order; portions outside the token ranges have no effect.
///
/// The returned ranges retain their original source coordinates and order. A token may
/// yield multiple fragments or disappear entirely if fully excluded. Fragments from
/// different tokens remain separate, even when adjacent.
///
/// For example, token ranges `[0..3, 3..6]` for `"ab->cd"` with exclusion `[2..4]`
/// produce `[0..2, 4..6]`, covering only `"ab"` and `"cd"`.
fn subtract_ranges(ranges: Vec<Range>, mut excluded: Vec<Range>) -> Vec<Range> {
    if excluded.is_empty() {
        return ranges;
    }
    // Different regexes may yield overlapping matches in any order. Sorting lets
    // us walk both sets once without needing to merge exclusions first.
    excluded.sort_unstable_by_key(|range| (range.start, range.end));
    let mut excluded = excluded.into_iter().peekable();
    let mut remaining = Vec::with_capacity(ranges.len());
    for range in ranges {
        let mut start = range.start;
        while let Some(matched) = excluded.peek() {
            if matched.end <= start {
                excluded.next();
                continue;
            }
            if matched.start >= range.end {
                break;
            }
            if start < matched.start {
                remaining.push(start..matched.start);
            }
            start = matched.end.min(range.end);
            if start == range.end {
                // Keep this exclusion: it may also overlap the next token.
                break;
            }
        }
        if start < range.end {
            remaining.push(start..range.end);
        }
    }
    remaining
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
                    .flatten()
                    .map(|m| {
                        let intra_word_range = m.start()..m.end();
                        log::trace!(target:"quirks",
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
                log::warn!(target:"quirks", "Matching regex >{}< errored: {}", regex.as_str(), e);
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

    #[test]
    fn dashed() {
        let _ = env_logger::builder()
            .is_test(true)
            .filter(None, log::LevelFilter::Trace)
            .try_init();

        const REPLACEMENTS: &[&str] = &["fffff", "qqq", "z", "zeta-ray"];
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
        const REPLACEMENTS: &[&str] = &["fffff", "qqq", "z", "zetaray"];
        assert!(replacements_contain_dashless(WORD, REPLACEMENTS));
    }

    #[test]
    fn cross_token_allow_lists_preserve_offsets_and_per_token_rules() {
        struct TestCase {
            name: &'static str,
            patterns: &'static [&'static str],
            text: &'static str,
            tokens: &'static [Range],
            expected: &'static [Range],
        }

        // These slices contain token ranges, not individual character offsets.
        #[allow(clippy::single_range_in_vec_init)]
        const TEST_CASES: &[TestCase] = &[
            TestCase {
                name: "Unicode offsets and partially matched tokens",
                patterns: &["->"],
                text: "éx->typo",
                tokens: &[0..3, 3..4, 4..8],
                expected: &[0..2, 4..8],
            },
            TestCase {
                name: "overlapping matches from different regexes",
                patterns: &["-+>", "->"],
                text: "---->",
                tokens: &[0..4, 4..5],
                expected: &[],
            },
            TestCase {
                name: "repeated adjacent matches",
                patterns: &["->"],
                text: "->->",
                tokens: &[0..1, 1..2, 2..3, 3..4],
                expected: &[],
            },
            TestCase {
                name: "anchors apply to individual non-whitespace sequences",
                patterns: &["^.+>$"],
                text: "typo ->",
                tokens: &[0..4, 5..6, 6..7],
                expected: &[0..4],
            },
            TestCase {
                name: "unrelated regex",
                patterns: &["::"],
                text: "->",
                tokens: &[0..1, 1..2],
                expected: &[0..1, 1..2],
            },
            TestCase {
                name: "zero-width match",
                patterns: &["(?=>)"],
                text: "->",
                tokens: &[0..1, 1..2],
                expected: &[0..1, 1..2],
            },
            TestCase {
                name: "captures remain subject to per-token transformation",
                patterns: &["(-)>"],
                text: "->",
                tokens: &[0..1, 1..2],
                expected: &[0..1, 1..2],
            },
            TestCase {
                name: "match within one of several tokens",
                patterns: &["typo"],
                text: "typoextra word",
                tokens: &[0..9, 10..14],
                expected: &[0..9, 10..14],
            },
            TestCase {
                name: "no regexes",
                patterns: &[],
                text: "->",
                tokens: &[0..1, 1..2],
                expected: &[0..1, 1..2],
            },
            TestCase {
                name: "no tokens",
                patterns: &["->"],
                text: "->",
                tokens: &[],
                expected: &[],
            },
            TestCase {
                name: "only one token",
                patterns: &["->"],
                text: "->",
                tokens: &[0..2],
                expected: &[0..2],
            },
            TestCase {
                name: "Unicode whitespace and leading whitespace preserve offsets",
                patterns: &["->"],
                text: " \té\u{2003}->\r\nword",
                tokens: &[2..3, 4..5, 5..6, 8..12],
                expected: &[2..3, 8..12],
            },
            TestCase {
                name: "separate matches preserve the intervening token fragment",
                patterns: &["->", "::"],
                text: "ab->mid::cd",
                tokens: &[0..3, 3..8, 8..11],
                expected: &[0..2, 4..7, 9..11],
            },
            TestCase {
                name: "partially overlapping matches preserve outer fragments",
                patterns: &["->:", ">::"],
                text: "ab->::cd",
                tokens: &[0..3, 3..4, 4..5, 5..8],
                expected: &[0..2, 6..8],
            },
        ];

        for case in TEST_CASES {
            let regexes = Vec::from_iter(
                case.patterns
                    .iter()
                    .map(|pattern| WrappedRegex::from(Regex::new(pattern).unwrap())),
            );
            assert_eq!(
                allow_list_ranges(&regexes, case.text, case.tokens.iter().cloned()),
                case.expected,
                "{}: {:?}: {:?}",
                case.name,
                case.patterns,
                case.text,
            );
        }
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

        let words = ["2x", r#"''so-to-speak''"#, "Alphabeta", "Nothing"];

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
