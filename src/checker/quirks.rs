//! A set of quirks, not necessarily specific to a checker

use crate::Range;
use fancy_regex::Regex;

#[cfg(test)]
mod tests;

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
