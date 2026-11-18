//! A set of quirks, not necessarily specific to a checker

use crate::Range;
use fancy_regex::Regex;
use lazy_static::lazy_static;

/// The value is `true` if string is made of emoji's or Unicode
/// `VULGAR FRACTION`.
pub(crate) fn consists_of_vulgar_fractions_or_emojis(word: &str) -> bool {
    lazy_static! {
        static ref VULGAR_OR_EMOJI: regex::RegexSet = regex::RegexSetBuilder::new([
            r"[\u00BC-\u00BE\u2150-\u215E-\u2189]",
            r"^[\p{Emoji}]+$"
        ])
        .case_insensitive(true)
        .build()
        .expect("REGEX grammar is human checked. qed");
    };
    VULGAR_OR_EMOJI.is_match(word)
}

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

/// Apply compound regex matches, retaining captures and unmatched token fragments.
///
/// Token ranges must be ordered and non-overlapping, with character (not byte) offsets.
/// Matches within a single token retain the existing per-token transformation policy.
/// An earlier regex matching an original token protects that entire token from later
/// compound transformations. Earlier exclusions cannot be restored by later captures.
/// Each nonempty capture is retained independently, even across token boundaries.
pub(crate) fn compound_ranges<R: AsRef<Regex>>(
    transform_regex: &[R],
    text: &str,
    ranges: impl Iterator<Item = Range>,
) -> Vec<Range> {
    let ranges = Vec::from_iter(ranges);
    if transform_regex.is_empty() || ranges.len() < 2 {
        return ranges;
    }
    // Regex matches use byte offsets, while tokenizer ranges use character offsets.
    // Including the final boundary also permits matches ending at the end of the text.
    let char_boundaries = Vec::from_iter(
        text.char_indices()
            .map(|(offset, _)| offset)
            .chain(std::iter::once(text.len())),
    );
    let mut excluded = Vec::<Range>::new();
    let mut captured = Vec::<Range>::new();
    let mut byte_offset = 0;
    // Keep word-oriented anchors and wildcards from swallowing neighboring words.
    for run in text.split_inclusive(char::is_whitespace) {
        let excluded_start = excluded.len();
        let captured_start = captured.len();
        let word = run.trim_end_matches(char::is_whitespace);
        let source_range = |start, end| {
            let start = char_boundaries
                .binary_search(&(byte_offset + start))
                .expect("regex matches start at a character boundary");
            let end = char_boundaries
                .binary_search(&(byte_offset + end))
                .expect("regex matches end at a character boundary");
            start..end
        };
        for (regex_index, regex) in transform_regex.iter().map(AsRef::as_ref).enumerate() {
            // Only earlier rules can protect this match; same-rule overlaps are rejected below.
            let protected = merge_ranges(
                excluded[excluded_start..]
                    .iter()
                    .chain(&captured[captured_start..])
                    .cloned(),
            );
            let mut previous_match_end = 0;
            for captures in regex.captures_iter(word) {
                let captures = match captures {
                    Ok(captures) => captures,
                    Err(error) => {
                        log::warn!(target: "quirks", "Matching regex >{}< errored: {}", regex.as_str(), error);
                        break;
                    }
                };
                let matched = captures.get(0).expect("captures include the full match");
                if matched.start() >= matched.end() || matched.start() < previous_match_end {
                    continue;
                }
                // `\K` in lookarounds can reset group 0 behind the iterator's position.
                previous_match_end = matched.end();
                let matched = source_range(matched.start(), matched.end());
                let first = ranges.partition_point(|range| range.end <= matched.start);
                // A second overlapping token distinguishes cross-token matches from
                // matches that must still be handled by `transform` per token.
                if !ranges
                    .get(first + 1)
                    .is_some_and(|range| range.start < matched.end)
                {
                    continue;
                }
                // Earlier runs cannot overlap this match; only copy this run's decisions.
                let first_protected = protected.partition_point(|range| range.end <= matched.start);
                let mut blocked = Vec::from_iter(
                    protected[first_protected..]
                        .iter()
                        .take_while(|range| range.start < matched.end)
                        .cloned(),
                );
                for token in &ranges[first..] {
                    if token.start >= matched.end {
                        break;
                    }
                    // Decide precedence on the original token, not surviving fragments.
                    let token_text =
                        &text[char_boundaries[token.start]..char_boundaries[token.end]];
                    // Errors also stop per-token transformation, which logs them later.
                    if transform_regex[..regex_index]
                        .iter()
                        .any(|earlier| earlier.as_ref().is_match(token_text).unwrap_or(true))
                    {
                        blocked.push(token.clone());
                    }
                }
                let eligible = subtract_ranges(vec![matched], blocked);
                let mut selected = Vec::new();
                for capture in captures.iter().skip(1).flatten() {
                    let capture = source_range(capture.start(), capture.end());
                    // Lookaround captures cannot pull text outside the compound match.
                    for range in &eligible {
                        let start = range.start.max(capture.start);
                        let end = range.end.min(capture.end);
                        if start < end {
                            selected.push(start..end);
                        }
                    }
                }
                excluded.extend(subtract_ranges(eligible, selected.clone()));
                captured.extend(selected);
            }
        }
        byte_offset += run.len();
    }
    // Captures replace covered token portions without merging distinct capture groups.
    let mut remaining = subtract_ranges(
        ranges,
        Vec::from_iter(excluded.into_iter().chain(captured.iter().cloned())),
    );
    remaining.extend(captured);
    remaining.sort_unstable_by_key(|range| (range.start, range.end));
    remaining.dedup();
    remaining
}

/// Merge protection intervals for ordered lookup, without merging capture candidates.
fn merge_ranges(ranges: impl IntoIterator<Item = Range>) -> Vec<Range> {
    let mut ranges = Vec::from_iter(ranges);
    ranges.sort_unstable_by_key(|range| (range.start, range.end));
    let mut merged = Vec::<Range>::with_capacity(ranges.len());
    for range in ranges {
        if let Some(previous) = merged
            .last_mut()
            .filter(|previous| range.start <= previous.end)
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
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
    let mut q = std::collections::VecDeque::<(Range, &'_ str, usize)>::with_capacity(32);
    let mut words = Vec::with_capacity(16);
    let mut whitelisted = 0usize;
    let mut visited = std::collections::HashSet::new();
    q.push_back((range.clone(), word, 0));
    while let Some((range, word, next_regex)) = q.pop_front() {
        // Nested captures can reach the same fragment and rule cursor by several paths.
        if !visited.insert((range.start, range.end, next_regex)) {
            continue;
        }
        // work on a fragment now
        let (matched_regex, transformed) =
            transform_inner(transform_regex, word, range.clone(), next_regex);
        match transformed {
            // we try to recursively match the fragments with the regex expr until they become atomic words or whitelisted
            Transformed::Fragments(v) => {
                q.extend(v.into_iter().map(|(fragment_range, fragment)| {
                    // An unchanged capture advances to later rules instead of looping forever.
                    let next_regex = if fragment_range == range {
                        matched_regex + 1
                    } else {
                        0
                    };
                    (fragment_range, fragment, next_regex)
                }))
            }
            Transformed::Atomic(word) => words.push(word),
            Transformed::Whitelisted(_) => whitelisted += 1,
        }
    }

    words.sort_unstable_by_key(|(range, _)| (range.start, range.end));
    words.dedup();
    // no match found at all, this word is "atomic" and will be checked as is
    if whitelisted == 0usize {
        // empty means nothing, one word with the same range means we only found the initial provided word
        if words.is_empty() || (words.len() == 1 && words[0].0 == range) {
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

/// Find the first matching rule at or after `next_regex` and return its transformation.
///
/// The matching index lets unchanged captures resume at the following rule.
fn transform_inner<'i, R: AsRef<Regex>>(
    transform_regex: &[R],
    word: &'i str,
    range: Range,
    next_regex: usize,
) -> (usize, Transformed<'i>) {
    for (regex_index, regex) in transform_regex
        .iter()
        .map(AsRef::as_ref)
        .enumerate()
        .skip(next_regex)
    {
        match regex.captures(word) {
            Ok(Some(captures)) => {
                // first one is always the full match
                if captures.len() == 1 {
                    // means match, but no captures,
                    // which is equiv to an implicit whitelist
                    return (regex_index, Transformed::Whitelisted((range, word)));
                }
                let intermediate = Vec::from_iter(captures
                    .iter()
                    .skip(1)
                    .flatten()
                    .filter(|m| m.start() < m.end())
                    .map(|m| {
                        let intra_word_range = m.start()..m.end();
                        log::trace!(target:"quirks",
                            "Found capture for word >{}<, with match >{}< and capture >{}< at {:?}",
                            captures.get(0).unwrap().as_str(),
                            word,
                            m.as_str(),
                            intra_word_range
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
                    }));

                let transformed = if intermediate.is_empty() {
                    Transformed::Atomic((range, word))
                } else {
                    Transformed::Fragments(intermediate)
                };
                return (regex_index, transformed);
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
    (transform_regex.len(), Transformed::Atomic((range, word)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WrappedRegex;

    macro_rules! parametrized_vulgar_fraction_or_emoji {
        ($($name:ident: $value:expr,)*) => {
        $(
            #[test]
            fn $name() {
                let (input, expected) = $value;
                assert_eq!(expected, consists_of_vulgar_fractions_or_emojis(input));
            }
        )*
        }
    }

    parametrized_vulgar_fraction_or_emoji! {
        empty: ("", false),
        emojis: ("🐍🤗🦀", true),
        contains_emojis: ("🦀acean", false),
        contains_only_unicode: ("⅔⅔⅔↉↉↉", true),
        contains_emojis_and_unicodes: ("🐍🤗⅒🦀⅔¾", true),
        no_emojis: ("no emoji string", false),
        is_number: ("123", true),
        is_latin_letter: ("a", false),
        vulgar_fraction_one_quarter_and_emojis: ("¼🤗🦀", true),
        emojis_and_vulgar_fraction_one_half: ("🤗🦀½", true),
        emojis_and_vulgar_fraction_three_quarters: ("🤗🦀¾", true),
        emojis_and_vulgar_fraction_one_seventh: ("🤗🦀⅐", true),
        emojis_and_vulgar_fraction_one_ninth: ("🤗🦀⅑", true),
        emojis_and_vulgar_fraction_one_tenth: ("🤗🦀⅒", true),
        emojis_and_vulgar_fraction_one_third: ("🤗🦀⅓", true),
        emojis_and_vulgar_fraction_two_thirds: ("🤗🦀⅔", true),
        emojis_and_vulgar_fraction_one_fifth: ("🤗🦀⅕", true),
        emojis_and_vulgar_fraction_two_fifth: ("🤗🦀⅖", true),
        emojis_and_vulgar_fraction_three_fifths: ("🤗🦀⅗", true),
        emojis_and_vulgar_fraction_four_fifths: ( "🐍⅘", true),
        emojis_and_vulgar_fraction_one_sixth: ("🐍⅙", true),
        emojis_and_vulgar_fraction_five_sixths: ("🐍⅚", true),
        emojis_and_vulgar_fraction_one_eighth: ("🦀🐍⅛", true),
        emojis_and_vulgar_fraction_three_eighths: ("🦀🐍⅜", true),
        emojis_and_vulgar_fraction_five_eights: ("🦀🐍⅝", true),
        emojis_and_vulgar_fraction_five_eighths: ("🦀🐍⅝", true),
        emojis_and_vulgar_fraction_seven_eighths: ("🦀🐍⅞", true),
        emojis_and_vulgar_fraction_zero_thirds: ("🦀🐍↉", true),
    }

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

    /// The first matching compound capture rule determines which fragment is checked.
    #[test]
    fn compound_capture_precedence_follows_regex_order() {
        /// Competing rules and the sole capture that must survive in each order.
        struct TestCase {
            patterns: [&'static str; 2],
            expected_range: Range,
        }

        let text = "reciever->mesage";
        let tokens = [0..9, 9..10, 10..16];
        let cases = [
            TestCase {
                patterns: ["(reciever)->mesage", "reciever->(mesage)"],
                expected_range: 0..8,
            },
            TestCase {
                patterns: ["reciever->(mesage)", "(reciever)->mesage"],
                expected_range: 10..16,
            },
        ];
        for case in cases {
            let patterns = case.patterns;
            let expected = [case.expected_range];
            let regexes = Vec::from_iter(
                patterns
                    .iter()
                    .map(|pattern| WrappedRegex::from(Regex::new(pattern).unwrap())),
            );
            let actual = compound_ranges(&regexes, text, tokens.iter().cloned());
            assert_eq!(actual, expected, "{patterns:?}: {text}");
        }
    }

    /// One compound regex selects every named capture without including its separators.
    #[test]
    fn compound_regex_retains_multiple_captures() {
        let text = "récepteur->message::notification";
        let tokens = [0..10, 10..11, 11..18, 18..19, 19..20, 20..32];
        let pattern = r"(?P<receiver>\p{L}+)->(?P<message>\p{L}+)::(?P<notification>\p{L}+)";
        let regexes = [WrappedRegex::from(Regex::new(pattern).unwrap())];
        let expected_ranges = [0..9, 11..18, 20..32];
        let expected_words = ["récepteur", "message", "notification"];

        let actual_ranges = compound_ranges(&regexes, text, tokens.into_iter());
        assert_eq!(actual_ranges, expected_ranges);
        let actual_words = Vec::from_iter(
            actual_ranges
                .into_iter()
                .map(|range| crate::sub_chars(text, range)),
        );
        assert_eq!(actual_words, expected_words);
    }

    /// Compound captures and exclusions preserve source coordinates and ordered rules.
    #[test]
    fn compound_ranges_preserve_offsets_and_rule_precedence() {
        struct TestCase {
            name: &'static str,
            patterns: &'static [&'static str],
            text: &'static str,
            tokens: &'static [Range],
            expected: &'static [Range],
        }

        // These slices contain token ranges, not individual character offsets.
        /// Cross-token matching, offsets, and ordered per-token rule interactions.
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
                name: "a compound capture selects its fragment across token boundaries",
                patterns: &["(-)>"],
                text: "->",
                tokens: &[0..1, 1..2],
                expected: &[0..1],
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
            TestCase {
                name: "review regression: earlier capture preserves its original token",
                patterns: &["(foooo)-", "foooo->"],
                text: "foooo->word",
                tokens: &[0..6, 6..7, 7..11],
                expected: &[0..6, 7..11],
            },
            TestCase {
                name: "earlier cross-token exclusion wins over a later capture",
                patterns: &["foooo->", "(foooo)-"],
                text: "foooo->word",
                tokens: &[0..6, 6..7, 7..11],
                expected: &[7..11],
            },
            TestCase {
                name: "a nonmatching earlier capture does not block exclusions",
                patterns: &["(reqeust)-", "reciever->"],
                text: "reciever->message",
                tokens: &[0..9, 9..10, 10..17],
                expected: &[10..17],
            },
            TestCase {
                name: "a partial exclusion cannot change an anchored capture's token",
                patterns: &["^(reciever)-$", "->"],
                text: "reciever->message",
                tokens: &[0..9, 9..10, 10..17],
                expected: &[0..9, 10..17],
            },
            TestCase {
                name: "a protected middle token does not protect its neighbors",
                patterns: &["(>)-", "ab->-cd"],
                text: "ab->-cd",
                tokens: &[0..3, 3..5, 5..7],
                expected: &[3..5],
            },
            TestCase {
                name: "earlier per-token allow-list rules also retain their full token",
                patterns: &["^reciever-$", "->"],
                text: "reciever->message",
                tokens: &[0..9, 9..10, 10..17],
                expected: &[0..9, 10..17],
            },
            TestCase {
                name: "precedence checks use character offsets with Unicode tokens",
                patterns: &["^(éreciever)-$", "reciever->"],
                text: "éreciever->message",
                tokens: &[0..10, 10..11, 11..18],
                expected: &[0..10, 11..18],
            },
            TestCase {
                name: "protection against a later rule does not undo an earlier exclusion",
                patterns: &["->", "^>(reciever):$", "->reciever::"],
                text: "ab->reciever::tail",
                tokens: &[0..3, 3..13, 13..18],
                expected: &[0..2, 4..13, 14..18],
            },
            TestCase {
                name: "multiple named captures preserve Unicode offsets and neighbors",
                patterns: &[r"\[(?P<recipient>reciever)::(?P<address>adres)\]"],
                text: "é[reciever::adres]+tail",
                tokens: &[0..2, 2..10, 10..11, 11..12, 12..17, 17..19, 19..23],
                expected: &[0..1, 2..10, 12..17, 18..19, 19..23],
            },
            TestCase {
                name: "captures can span several original tokens",
                patterns: &[r"\[(reciever::adres)\](\+tail)"],
                text: "é[reciever::adres]+tail",
                tokens: &[0..2, 2..10, 10..11, 11..12, 12..17, 17..19, 19..23],
                expected: &[0..1, 2..17, 18..23],
            },
            TestCase {
                name: "nested identical capture ranges are checked only once",
                patterns: &[r"\[((reciever))::(adres)\]"],
                text: "[reciever::adres]",
                tokens: &[0..1, 1..9, 9..10, 10..11, 11..16, 16..17],
                expected: &[1..9, 11..16],
            },
            TestCase {
                name: "distinct overlapping groups remain independent candidates",
                patterns: &[r"\[((reciever)::(adres))\]"],
                text: "[reciever::adres]",
                tokens: &[0..1, 1..9, 9..10, 10..11, 11..16, 16..17],
                expected: &[1..9, 1..16, 11..16],
            },
            TestCase {
                name: "earlier compound captures survive later cross-token exclusions",
                patterns: &[r"\[(reciever)::(adres)\]", "reciever::adres"],
                text: "[reciever::adres]",
                tokens: &[0..1, 1..9, 9..10, 10..11, 11..16, 16..17],
                expected: &[1..9, 11..16],
            },
            TestCase {
                name: "later captures cannot restore an earlier exclusion",
                patterns: &["reciever::adres", r"\[(reciever)::(adres)\]"],
                text: "[reciever::adres]",
                tokens: &[0..1, 1..9, 9..10, 10..11, 11..16, 16..17],
                expected: &[],
            },
            TestCase {
                name: "an earlier exclusion splits a spanning capture without restoring text",
                patterns: &["::", r"\[(reciever::adres)\]"],
                text: "[reciever::adres]",
                tokens: &[0..1, 1..9, 9..10, 10..11, 11..16, 16..17],
                expected: &[1..9, 11..16],
            },
            TestCase {
                name: "an earlier per-token rule protects its token from compound captures",
                patterns: &["^(reciever)-$", "(reciever)->(mesage)"],
                text: "reciever->mesage",
                tokens: &[0..9, 9..10, 10..16],
                expected: &[0..9, 10..16],
            },
            TestCase {
                name: "absent captures leave no candidates inside the compound match",
                patterns: &[r"\[(adres)?\]"],
                text: "[]",
                tokens: &[0..1, 1..2],
                expected: &[],
            },
            TestCase {
                name: "lookaround captures are clipped to the full compound match",
                patterns: &[r"(?<=(pre))(-)>(?=(tail))"],
                text: "pre->tail",
                tokens: &[0..4, 4..5, 5..9],
                expected: &[0..3, 3..4, 5..9],
            },
            TestCase {
                name: "repeated matches retain separate captures and intervening punctuation",
                patterns: &[r"([a-z])->([a-z])"],
                text: "a->b,c->d",
                tokens: &[0..2, 2..3, 3..5, 5..7, 7..8, 8..9],
                expected: &[0..1, 3..4, 4..5, 5..6, 8..9],
            },
            TestCase {
                name: "source-reset lookbehinds cannot restore overlapping exclusions",
                patterns: &[r"(?<=\K(.).)."],
                text: "a->b->c",
                tokens: &[0..2, 2..3, 3..5, 5..6, 6..7],
                expected: &[0..1, 3..4, 6..7],
            },
        ];

        for case in TEST_CASES {
            let regexes = Vec::from_iter(
                case.patterns
                    .iter()
                    .map(|pattern| WrappedRegex::from(Regex::new(pattern).unwrap())),
            );
            let text = case.text;
            let tokens = case.tokens;
            let expected = case.expected;
            let actual = compound_ranges(&regexes, text, tokens.iter().cloned());
            assert_eq!(
                actual, expected,
                "{}: {:?}: {:?}",
                case.name, case.patterns, case.text,
            );
        }
    }

    /// Recursive captures must shrink the fragment or advance to a later rule.
    #[test]
    fn capture_recursion_makes_progress() {
        /// Ordered capture rules and their final source-coordinate transformation.
        struct TestCase {
            patterns: &'static [&'static str],
            text: &'static str,
            range: Range,
            expected: Transformed<'static>,
        }
        let cases = [
            TestCase {
                patterns: &["^(.+)$"],
                text: "éreciever",
                range: 10..19,
                expected: Transformed::Atomic((10..19, "éreciever")),
            },
            TestCase {
                patterns: &["^(.+)$", "^receiver$"],
                text: "receiver",
                range: 4..12,
                expected: Transformed::Whitelisted((4..12, "receiver")),
            },
            TestCase {
                patterns: &["^'(.+)'$"],
                text: "''scheduel''",
                range: 5..17,
                expected: Transformed::Fragments(vec![(7..15, "scheduel")]),
            },
            TestCase {
                patterns: &["^((reciever))$"],
                text: "reciever",
                range: 2..10,
                expected: Transformed::Atomic((2..10, "reciever")),
            },
            TestCase {
                patterns: &["^((reciever)(mesage))$"],
                text: "recievermesage",
                range: 0..14,
                expected: Transformed::Fragments(vec![
                    (0..8, "reciever"),
                    (0..14, "recievermesage"),
                    (8..14, "mesage"),
                ]),
            },
            TestCase {
                patterns: &["^ticket(adres)?$"],
                text: "ticket",
                range: 0..6,
                expected: Transformed::Atomic((0..6, "ticket")),
            },
            TestCase {
                patterns: &["(tikcet)->(reciever)", "^tikcet(adres)?$"],
                text: "tikcet->reciever",
                range: 0..16,
                expected: Transformed::Fragments(vec![(0..6, "tikcet"), (8..16, "reciever")]),
            },
        ];
        for case in cases {
            let text = case.text;
            let range = case.range;
            let expected = case.expected;
            let regexes = Vec::from_iter(
                case.patterns
                    .iter()
                    .map(|pattern| WrappedRegex::from(Regex::new(pattern).unwrap())),
            );
            let actual = transform(&regexes, text, range);
            assert_eq!(actual, expected, "{:?}: {text}", case.patterns);
        }
    }

    /// Repeated matches stay independent without rebuilding a growing match history.
    #[test]
    fn repeated_compound_matches_respect_precedence() {
        let patterns = ["->", "(-)>"];
        let regexes = Vec::from_iter(
            patterns
                .iter()
                .map(|pattern| WrappedRegex::from(Regex::new(pattern).unwrap())),
        );
        let repeated_match_count = 16_000;
        let text = "->".repeat(repeated_match_count);
        let tokens = (0..text.len()).map(|start| start..start + 1);
        let expected = Vec::<Range>::new();
        let actual = compound_ranges(&regexes, &text, tokens);
        assert_eq!(actual, expected);
    }

    /// Duplicate identity captures must not multiply recursive work at each rule.
    #[test]
    fn repeated_identity_captures_are_deduplicated() {
        let text = "notifcation";
        let range = 7..18;
        let pattern = "^((.+))$";
        let repeated_rule_count = 32;
        let regexes = Vec::from_iter(
            (0..repeated_rule_count).map(|_| WrappedRegex::from(Regex::new(pattern).unwrap())),
        );
        let expected = Transformed::Atomic((range.clone(), text));
        let actual = transform(&regexes, text, range);
        assert_eq!(actual, expected);
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
