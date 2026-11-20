use super::*;
use crate::load_span_from;
use crate::ContentOrigin;
use crate::Documentation;
use crate::LineColumn;
use crate::Range;
use crate::Span;
use std::path::PathBuf;

use crate::fluff_up;

fn alternate_chunk(prefix: &str, source_line: usize) -> CheckableChunk {
    let start = prefix.chars().count() + 1;
    CheckableChunk::from_string(
        format!("{}\ntypoo", prefix),
        indexmap::indexmap! {
            start..start + 5 => Span {
                start: LineColumn { line: source_line, column: 4 },
                end: LineColumn { line: source_line, column: 8 },
            }
        },
        crate::CommentVariant::TripleSlash,
    )
}

fn shared_prose_suggestion(chunk: &CheckableChunk) -> Suggestion<'_> {
    let end = chunk.len_in_chars();
    let range = end - 5..end;
    let span = *chunk.find_spans(range.clone()).first().unwrap().1;
    Suggestion {
        detector: Detector::Hunspell,
        origin: ContentOrigin::TestEntityRust,
        chunk,
        span,
        range,
        replacements: vec!["typo".to_owned()],
        description: Some("Possible spelling mistake found.".to_owned()),
    }
}

#[test]
fn deduplicates_shared_prose_across_alternative_chunks() {
    let first = alternate_chunk("Unix", 10);
    let alternate = alternate_chunk("Other platform", 10);
    let original = shared_prose_suggestion(&first);
    let duplicate = shared_prose_suggestion(&alternate);
    assert_ne!(original.range, duplicate.range);
    let mut other_diagnostic = original.clone();
    other_diagnostic.detector = Detector::NlpRules;
    let mut suggestions = vec![original, other_diagnostic, duplicate];
    suggestions.sort();

    deduplicate_by_source(&mut suggestions);

    assert_eq!(suggestions.len(), 2);
    assert!(std::ptr::eq(suggestions[0].chunk, &first));
    assert_eq!(suggestions[0].detector, Detector::Hunspell);
    assert_eq!(suggestions[1].detector, Detector::NlpRules);
}

#[test]
fn preserves_identical_prose_at_distinct_source_spans() {
    let first = alternate_chunk("Unix", 10);
    let second = alternate_chunk("Unix", 11);
    let mut suggestions = vec![
        shared_prose_suggestion(&second),
        shared_prose_suggestion(&first),
    ];
    suggestions.sort();

    deduplicate_by_source(&mut suggestions);

    assert_eq!(suggestions.len(), 2);
    assert_eq!(suggestions[0].span.start.line, 10);
    assert_eq!(suggestions[1].span.start.line, 11);
}

#[test]
fn preserves_different_diagnostics_at_the_same_span() {
    let chunk = alternate_chunk("Unix", 10);
    let original = shared_prose_suggestion(&chunk);
    let mut other_origin = original.clone();
    other_origin.origin = ContentOrigin::TestEntityCommonMark;
    let mut other_detector = original.clone();
    other_detector.detector = Detector::NlpRules;
    let mut other_replacement = original.clone();
    other_replacement.replacements = vec!["type".to_owned()];
    let mut other_description = original.clone();
    other_description.description = Some("A different diagnostic.".to_owned());
    let mut suggestions = vec![
        original,
        other_origin,
        other_detector,
        other_replacement,
        other_description,
    ];
    suggestions.sort();
    let expected = suggestions.clone();

    deduplicate_by_source(&mut suggestions);

    assert!(suggestions == expected);
}

const TEXT: &str = "With markdown removed, for sure.";
lazy_static::lazy_static! {
    static ref TOKENS: Vec<&'static str> = vec![
        "With",
        "markdown",
        "removed",
        ",",
        "for",
        "sure",
        ".",
    ];
}

#[test]
fn tokens() {
    let tokenizer = tokenizer::<&PathBuf>(None).unwrap();
    let ranges: Vec<Range> = dbg!(apply_tokenizer(&tokenizer, TEXT).collect());
    for (range, expect) in ranges.into_iter().zip(TOKENS.iter()) {
        assert_eq!(&&TEXT[range], expect);
    }
}

pub fn extraction_test_body(content: &str, expected_spans: &[Span]) {
    let _ = env_logger::builder()
        .filter(None, log::LevelFilter::Trace)
        .is_test(true)
        .try_init();
    let doc_comments = true;
    let dev_comments = false;
    let docs = Documentation::load_from_str(
        ContentOrigin::TestEntityRust,
        content,
        doc_comments,
        dev_comments,
    );
    let (origin, chunks) = docs.into_iter().next().expect("Contains exactly one file");
    let suggestions = dummy::DummyChecker
        .check(&origin, &chunks[..])
        .expect("Dummy extraction must never fail");

    // with a known number of suggestions
    assert_eq!(suggestions.len(), expected_spans.len());

    for (index, (suggestion, expected_span)) in
        suggestions.iter().zip(expected_spans.iter()).enumerate()
    {
        assert_eq!(
            suggestion.replacements,
            vec![format!("replacement_{index}")],
            "found vs expected replacement"
        );
        let extracts = load_span_from(&mut content.as_bytes(), suggestion.span).unwrap();
        let expected_extracts = load_span_from(&mut content.as_bytes(), *expected_span).unwrap();
        assert_eq!(
            (suggestion.span, extracts),
            (*expected_span, expected_extracts),
            "found vs expected span"
        );
    }
}

#[test]
fn extract_suggestions_simple() {
    const SIMPLE: &str = fluff_up!("two literals");

    /// keep in mind, `Span` bounds are inclusive, unlike Ranges, where
    /// `range.end` is _exclusive_
    const EXPECTED_SPANS: &[Span] = &[
        Span {
            start: LineColumn { line: 1, column: 4 },
            end: LineColumn { line: 1, column: 6 },
        },
        Span {
            start: LineColumn { line: 1, column: 8 },
            end: LineColumn {
                line: 1,
                column: 15,
            },
        },
    ];
    extraction_test_body(dbg!(SIMPLE), EXPECTED_SPANS);
}

#[test]
fn extract_suggestions_left_aligned() {
    const SIMPLE: &str = fluff_up!("two  literals ");

    /// keep in mind, `Span` bounds are inclusive, unlike Ranges, where
    /// `range.end` is _exclusive_
    const EXPECTED_SPANS: &[Span] = &[
        Span {
            start: LineColumn { line: 1, column: 4 },
            end: LineColumn { line: 1, column: 6 },
        },
        Span {
            start: LineColumn { line: 1, column: 9 },
            end: LineColumn {
                line: 1,
                column: 16,
            },
        },
    ];
    extraction_test_body(dbg!(SIMPLE), EXPECTED_SPANS);
}

#[test]
fn extract_suggestions_3spaces() {
    const SIMPLE: &str = fluff_up!("  third  testcase ");

    /// keep in mind, `Span` bounds are inclusive, unlike Ranges, where
    /// `range.end` is _exclusive_
    const EXPECTED_SPANS: &[Span] = &[
        Span {
            start: LineColumn { line: 1, column: 6 },
            end: LineColumn {
                line: 1,
                column: 10,
            },
        },
        Span {
            start: LineColumn {
                line: 1,
                column: 13,
            },
            end: LineColumn {
                line: 1,
                column: 20,
            },
        },
    ];
    extraction_test_body(dbg!(SIMPLE), EXPECTED_SPANS);
}

/// Build one spelling backend with built-in dictionaries and ordered regex rules.
#[cfg(any(feature = "hunspell", feature = "spellbook", feature = "zet"))]
fn spelling_checkers_for_regex(detector: Detector, patterns: &[&str]) -> Checkers {
    use crate::config::{HunspellConfig, Quirks, WrappedRegex};

    let transform_regex = Vec::from_iter(
        patterns
            .iter()
            .map(|pattern| WrappedRegex::from(fancy_regex::Regex::new(pattern).unwrap())),
    );
    let spelling = HunspellConfig {
        // Keep the regression independent of dictionaries installed on the host.
        use_builtin: true,
        skip_os_lookups: true,
        extra_dictionaries: vec![],
        quirks: Quirks {
            transform_regex,
            ..Default::default()
        },
        ..Default::default()
    };
    Checkers::new(Config {
        hunspell: (detector == Detector::Hunspell).then(|| spelling.clone()),
        spellbook: (detector == Detector::Spellbook).then(|| spelling.clone()),
        zet: (detector == Detector::ZSpell).then_some(spelling),
        nlprules: None,
        ..Default::default()
    })
    .unwrap()
}

#[cfg(any(feature = "hunspell", feature = "spellbook", feature = "zet"))]
fn assert_adjacent_spelling_diagnostics(
    checkers: &Checkers,
    detector: Detector,
    pattern: &str,
    words: [&str; 2],
    separator: &str,
) {
    let [left, right] = words;
    let source = format!("/// {left}{separator}{right}\nstruct Example;");
    let documents =
        Documentation::load_from_str(ContentOrigin::TestEntityRust, &source, true, false);
    let (origin, chunks) = documents.iter().next().unwrap();
    let suggestions = checkers.check(origin, chunks).unwrap();
    let context = format!("{detector}: {pattern}: {source}");
    assert_eq!(suggestions.len(), words.len(), "{context}");
    for (suggestion, word) in suggestions.iter().zip(words) {
        assert_eq!(suggestion.detector, detector, "{context}");
        assert_eq!(&suggestion.origin, origin, "{context}");
        let chunk = suggestion.chunk.as_str();
        // Both offsets count characters, not bytes, even next to a Unicode word.
        let start = chunk[..chunk.find(word).unwrap()].chars().count();
        let length = word.chars().count();
        assert_eq!(suggestion.range, start..start + length, "{context}");
        assert_eq!(
            crate::sub_chars(chunk, suggestion.range.clone()),
            word,
            "{context}"
        );
        let column = source[..source.find(word).unwrap()].chars().count();
        assert_eq!(
            suggestion.span,
            Span {
                start: LineColumn { line: 1, column },
                // Source spans have inclusive ends; chunk ranges are exclusive.
                end: LineColumn {
                    line: 1,
                    column: column + length - 1,
                },
            },
            "{context}"
        );
        assert_eq!(
            load_span_from(&mut source.as_bytes(), suggestion.span).unwrap(),
            word,
            "{context}"
        );
    }
}

#[cfg(any(feature = "hunspell", feature = "spellbook", feature = "zet"))]
fn check_adjacent_spelling_cases(
    checkers: &Checkers,
    detector: Detector,
    pattern: &str,
    separators: &[&str],
) {
    // Put the Unicode word on either side to exercise offsets across the match.
    const TEST_CASES: &[[&str; 2]] = &[
        ["Zzyzxqvocab", "zzyzxqvocabé"],
        ["Zzyzxqvocabé", "zzyzxqvocab"],
    ];
    for &words in TEST_CASES {
        for separator in separators {
            for padded_separator in [
                format!(" {separator} "),
                separator.to_string(),
                format!(" {separator}"),
                format!("{separator} "),
            ] {
                assert_adjacent_spelling_diagnostics(
                    checkers,
                    detector,
                    pattern,
                    words,
                    &padded_separator,
                );
            }
        }
    }
}

#[cfg(any(feature = "hunspell", feature = "spellbook", feature = "zet"))]
#[test]
fn no_capture_transform_regex_preserves_adjacent_spelling_diagnostics() {
    const DETECTORS: &[Detector] = &[
        #[cfg(feature = "hunspell")]
        Detector::Hunspell,
        #[cfg(feature = "spellbook")]
        Detector::Spellbook,
        #[cfg(feature = "zet")]
        Detector::ZSpell,
    ];
    const TEST_CASES: &[(&str, &[&str])] = &[
        ("->", &["->"]),
        ("-+>", &["->", "-->", "---->"]),
        ("::", &["::"]),
        ("<-", &["<-"]),
        ("[-=]>", &["->", "=>"]),
        ("[-=]+>", &["->", "=>", "==>", "--->"]),
    ];
    for &detector in DETECTORS {
        for &(pattern, separators) in TEST_CASES {
            let checkers = spelling_checkers_for_regex(detector, &[pattern]);
            check_adjacent_spelling_cases(&checkers, detector, pattern, separators);
        }

        // An unrelated regex must not change existing plain-arrow diagnostics.
        let source = "/// Zzyzxqvocab->zzyzxqvocabé\nstruct Example;";
        let documents =
            Documentation::load_from_str(ContentOrigin::TestEntityRust, source, true, false);
        let (origin, chunks) = documents.iter().next().unwrap();
        let baseline = spelling_checkers_for_regex(detector, &[])
            .check(origin, chunks)
            .unwrap();
        let unrelated = spelling_checkers_for_regex(detector, &["::"])
            .check(origin, chunks)
            .unwrap();
        assert!(!baseline.is_empty(), "{detector}: {source}");
        assert_eq!(
            unrelated, baseline,
            "{detector}: unrelated :: regex: {source}"
        );
    }
}

/// Check rule precedence, dictionary corrections, and Unicode diagnostic offsets.
#[cfg(any(feature = "hunspell", feature = "spellbook", feature = "zet"))]
#[test]
fn transform_regex_precedence_preserves_spelling_diagnostics() {
    /// An arrow's captured typo and its independently checked neighboring typo.
    struct PrecedenceCase {
        source: &'static str,
        patterns: [&'static str; 2],
        // Each pair is (typo, correction), with the captured word first.
        words: [(&'static str, &'static str); 2],
    }

    let cases = [
        PrecedenceCase {
            source: r#"/// 🦀 reciever->mesage
struct MessageRoute;"#,
            patterns: ["(reciever)-", "reciever->"],
            words: [("reciever", "receiver"), ("mesage", "message")],
        },
        PrecedenceCase {
            source: r#"/// 🦉 reqeust->respnse
struct RequestRoute;"#,
            patterns: ["(reqeust)-", "reqeust->"],
            words: [("reqeust", "request"), ("respnse", "response")],
        },
    ];
    let detectors = [
        #[cfg(feature = "hunspell")]
        Detector::Hunspell,
        #[cfg(feature = "spellbook")]
        Detector::Spellbook,
        #[cfg(feature = "zet")]
        Detector::ZSpell,
    ];
    let rule_orders = [true, false];
    for capture_first in rule_orders {
        for detector in detectors {
            for case in &cases {
                let source = case.source;
                let mut patterns = case.patterns;
                if !capture_first {
                    patterns.reverse();
                }
                let expected_words = if capture_first {
                    &case.words[..]
                } else {
                    &case.words[1..]
                };
                let checkers = spelling_checkers_for_regex(detector, &patterns);
                let context = format!("{detector}: {patterns:?}: {source}");

                let documents = Documentation::load_from_str(
                    ContentOrigin::TestEntityRust,
                    source,
                    true,
                    false,
                );
                let (origin, chunks) = documents.iter().next().unwrap();
                let suggestions = checkers.check(origin, chunks).unwrap();
                let expected_count = expected_words.len();
                assert_eq!(
                    suggestions.len(),
                    expected_count,
                    "{context}: {suggestions:?}"
                );
                for (suggestion, &(typo, correction)) in suggestions.iter().zip(expected_words) {
                    let chunk = suggestion.chunk.as_str();
                    let start = chunk[..chunk.find(typo).unwrap()].chars().count();
                    let length = typo.chars().count();
                    let expected_range = start..start + length;
                    assert_eq!(suggestion.range, expected_range, "{context}");
                    assert_eq!(
                        crate::sub_chars(chunk, suggestion.range.clone()),
                        typo,
                        "{context}"
                    );
                    assert_eq!(
                        load_span_from(&mut source.as_bytes(), suggestion.span).unwrap(),
                        typo,
                        "{context}"
                    );
                    if detector == Detector::Hunspell {
                        assert!(
                            suggestion
                                .replacements
                                .iter()
                                .any(|word| word == correction),
                            "{context}: expected {correction}: {:?}",
                            suggestion.replacements
                        );
                    }
                }
            }
        }
    }
}

/// Check compound captures, recursive transformations, and ordered exclusions on every backend.
#[cfg(any(feature = "hunspell", feature = "spellbook", feature = "zet"))]
#[test]
fn compound_transform_regex_captures_preserve_spelling_diagnostics() {
    /// Ordered rules and the spelling diagnostics expected in source order.
    struct CompoundCase {
        name: &'static str,
        source: &'static str,
        patterns: &'static [&'static str],
        // Each pair is (typo, dictionary correction).
        expected_words: &'static [(&'static str, &'static str)],
    }

    let cases = [
        CompoundCase {
            name: "independent captures exclude the uncaptured literal marker",
            source: r#"/// 🦀 CACHE::reciever->IGNROED->mesage::ID;occured
struct CacheMessage;"#,
            patterns: &[r"CACHE::(reciever)->IGNROED->(mesage)::ID"],
            expected_words: &[
                ("reciever", "receiver"),
                ("mesage", "message"),
                ("occured", "occurred"),
            ],
        },
        CompoundCase {
            name: "three punctuation-separated captures preserve neighboring diagnostics",
            source: r#"/// 🦉 succesful EVENT::conection,adress;lenght::END
struct ConnectionEvent;"#,
            patterns: &[r"EVENT::(conection),(adress);(lenght)::END"],
            expected_words: &[
                ("succesful", "successful"),
                ("conection", "connection"),
                ("adress", "address"),
                ("lenght", "length"),
            ],
        },
        CompoundCase {
            name: "a spanning capture is split recursively by a later rule",
            source: r#"/// 🐙 ROUTE::reqeust->respnse::END resutl
struct RequestResponse;"#,
            patterns: &[
                r"ROUTE::(reqeust->respnse)::END",
                r"(reqeust)->(respnse)",
                // Identity captures must terminate without hiding the captured typo.
                r"(reqeust)",
            ],
            expected_words: &[
                ("reqeust", "request"),
                ("respnse", "response"),
                ("resutl", "result"),
            ],
        },
        CompoundCase {
            name: "a spanning capture also recurses through earlier rules",
            source: r#"/// 🐢 TRACE::packte->timout::END warnign
struct PacketTrace;"#,
            patterns: &[r"(packte)->(timout)", r"TRACE::(packte->timout)::END"],
            expected_words: &[
                ("packte", "packet"),
                ("timout", "timeout"),
                ("warnign", "warning"),
            ],
        },
        CompoundCase {
            name: "an earlier per-token capture protects the original token",
            source: r#"/// 🦋 DOC::documnt->calback::END unkown
struct DocumentCallback;"#,
            patterns: &[r"(documnt)-", r"DOC::documnt->(calback)::END"],
            expected_words: &[
                ("documnt", "document"),
                ("calback", "callback"),
                ("unkown", "unknown"),
            ],
        },
        CompoundCase {
            name: "earlier cross-token captures survive a later exclusion",
            source: r#"/// 🐝 LINK::mispelled->paramter::END commited
struct ParameterLink;"#,
            patterns: &[
                r"LINK::(mispelled)->(paramter)::END",
                r"LINK::mispelled->paramter::END",
            ],
            expected_words: &[
                ("mispelled", "misspelled"),
                ("paramter", "parameter"),
                ("commited", "committed"),
            ],
        },
        CompoundCase {
            name: "an earlier cross-token exclusion is not restored by captures",
            source: r#"/// 🐬 PACKET::paylod->cheksum::END delivry
struct PacketDelivery;"#,
            patterns: &[
                r"PACKET::paylod->cheksum::END",
                r"PACKET::(paylod)->(cheksum)::END",
            ],
            expected_words: &[("delivry", "delivery")],
        },
        CompoundCase {
            name: "a partial earlier exclusion does not hide a later independent capture",
            source: r#"/// 🦎 BLOCK::threashold->notificaton::END interupt
struct ThresholdNotification;"#,
            patterns: &[
                r"BLOCK::threashold->",
                r"BLOCK::(threashold)->(notificaton)::END",
            ],
            expected_words: &[("notificaton", "notification"), ("interupt", "interrupt")],
        },
        CompoundCase {
            name: "nonparticipating optional captures and empty captures are ignored",
            source: r#"/// 🦜 QUEUE::notifcation::END subscripton
struct QueueSubscription;"#,
            patterns: &[r"QUEUE::(notifcation)(?:->(acknowlegement))?()::END"],
            expected_words: &[
                ("notifcation", "notification"),
                ("subscripton", "subscription"),
            ],
        },
        CompoundCase {
            name: "nested compound candidates cannot hide a disjoint sibling diagnostic",
            source: r#"/// 🦩 NEST::seperate::argumnt::END licnese
struct NestedArgument;"#,
            patterns: &[r"NEST::((seperate)::(argumnt))::END"],
            expected_words: &[
                ("seperate", "separate"),
                ("argumnt", "argument"),
                ("licnese", "license"),
            ],
        },
        CompoundCase {
            name: "nested captures do not duplicate diagnostics after recursive splitting",
            source: r#"/// 🦦 NEST::reciept->attachement::END messsage
struct NestedMessage;"#,
            patterns: &[
                r"NEST::((reciept)->(attachement))::END",
                r"(reciept)->(attachement)",
            ],
            expected_words: &[
                ("reciept", "receipt"),
                ("attachement", "attachment"),
                ("messsage", "message"),
            ],
        },
        CompoundCase {
            name: "an absent optional capture preserves its current recursive branch",
            source: r#"/// 🐌 WRAP::tikcet->reciever::END
struct TicketReceipt;"#,
            patterns: &[
                r"WRAP::(tikcet->reciever)::END",
                r"(tikcet)->(reciever)",
                r"^tikcet(adres)?$",
            ],
            expected_words: &[("tikcet", "ticket"), ("reciever", "receiver")],
        },
        CompoundCase {
            name: "captures sharing one source character cannot yield conflicting suggestions",
            source: r#"/// 🐢 NEST::recievermesage::END
struct SharedCharacter;"#,
            patterns: &[r"NEST::(?=(reciever))recieve(rmesage)::END"],
            expected_words: &[("reciever", "receiver")],
        },
        CompoundCase {
            name: "adjacent captures without a shared character retain both diagnostics",
            source: r#"/// 🦔 COMMAND::sucsessfailuer::END
struct CommandResult;"#,
            patterns: &[r"COMMAND::(sucsess)(failuer)::END"],
            expected_words: &[("sucsess", "success"), ("failuer", "failure")],
        },
        CompoundCase {
            name: "spanning captures may be recursively allow-listed as standalone fragments",
            source: r#"/// 🐳 SLOT::argumnt::licnese::END occurrance
struct LicensedArgument;"#,
            patterns: &[r"SLOT::(argumnt::licnese)::END", r"argumnt::licnese"],
            expected_words: &[("occurrance", "occurrence")],
        },
        CompoundCase {
            name: "later per-fragment checking uses the boundaries left by compound matching",
            source: r#"/// 🦀 reciever->adres
struct ReceiverAddress;"#,
            patterns: &[r"reciever(?!-)", r"->(adres)"],
            expected_words: &[("adres", "address")],
        },
        CompoundCase {
            name: "cross-token matches cannot span whitespace-separated runs",
            source: r#"/// 🦊 whille :: untill
struct LoopBoundary;"#,
            patterns: &[r"whille\s+::\s+(untill)"],
            expected_words: &[("whille", "while"), ("untill", "until")],
        },
    ];
    let detectors = [
        #[cfg(feature = "hunspell")]
        Detector::Hunspell,
        #[cfg(feature = "spellbook")]
        Detector::Spellbook,
        #[cfg(feature = "zet")]
        Detector::ZSpell,
    ];
    for detector in detectors {
        for case in &cases {
            let source = case.source;
            let patterns = case.patterns;
            let expected_words = case.expected_words;
            let checkers = spelling_checkers_for_regex(detector, patterns);
            let context = format!("{detector}: {}: {patterns:?}: {source}", case.name);
            let documents =
                Documentation::load_from_str(ContentOrigin::TestEntityRust, source, true, false);
            let (origin, chunks) = documents.iter().next().unwrap();
            let suggestions = checkers.check(origin, chunks).unwrap();
            let expected_count = expected_words.len();
            assert_eq!(
                suggestions.len(),
                expected_count,
                "{context}: {suggestions:?}"
            );
            for (suggestion, &(typo, correction)) in suggestions.iter().zip(expected_words) {
                let chunk = suggestion.chunk.as_str();
                let start = chunk[..chunk.find(typo).unwrap()].chars().count();
                let length = typo.chars().count();
                let expected_range = start..start + length;
                let column = source[..source.find(typo).unwrap()].chars().count();
                let expected_span = Span {
                    start: LineColumn { line: 1, column },
                    end: LineColumn {
                        line: 1,
                        column: column + length - 1,
                    },
                };
                assert_eq!(suggestion.detector, detector, "{context}");
                assert_eq!(&suggestion.origin, origin, "{context}");
                assert_eq!(suggestion.range, expected_range, "{context}");
                assert_eq!(suggestion.span, expected_span, "{context}");
                assert_eq!(
                    crate::sub_chars(chunk, suggestion.range.clone()),
                    typo,
                    "{context}"
                );
                assert_eq!(
                    load_span_from(&mut source.as_bytes(), suggestion.span).unwrap(),
                    typo,
                    "{context}"
                );
                if detector == Detector::Hunspell {
                    assert!(
                        suggestion
                            .replacements
                            .iter()
                            .any(|word| word == correction),
                        "{context}: expected {correction}: {:?}",
                        suggestion.replacements
                    );
                }
            }
        }
    }
}

#[test]
fn checker_discrepancies() {
    let _ = env_logger::Builder::new()
        .default_format()
        .filter_level(log::LevelFilter::Debug)
        .filter(Some("dicaff"), log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    let x = r###"
/// With all patches applied.
///
/// No line in need of a reflow.
///
/// `Patch`s foo.
///
/// I am a TODO where TODO is in the extra dictionary.
struct X;
"###;

    let mut doc = Documentation::new();
    doc.add_rust(ContentOrigin::TestEntityRust, x, true, false)
        .unwrap();

    let config = Config::default();
    assert!(config.is_enabled(Detector::Hunspell));
    assert!(config.is_enabled(Detector::Spellbook));
    assert!(config.is_enabled(Detector::ZSpell));
    let cs = Checkers::new(config).unwrap();

    let (origin, ccs) = doc.iter().next().unwrap();
    dbg!(&ccs);
    let assert_cmp = |a: &[Suggestion<'_>], b: &[Suggestion<'_>]| {
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b.iter()) {
            assert_eq!(a.range, b.range);
            assert_eq!(a.chunk, b.chunk);
        }
    };

    let hun = dbg!(cs.hunspell.unwrap().check(origin, ccs)).unwrap();
    let book = dbg!(cs.spellbook.unwrap().check(origin, ccs)).unwrap();
    let z = dbg!(cs.zet.unwrap().check(origin, ccs)).unwrap();
    assert_cmp(&hun, &z);
    assert_cmp(&z, &book);
}
