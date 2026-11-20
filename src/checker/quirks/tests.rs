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
