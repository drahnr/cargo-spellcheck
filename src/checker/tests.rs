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
        checked_word: Some("typoo".to_owned()),
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
    let (source, expected_markdown) = (
        format!("/// {left}{separator}{right}\nstruct Example;"),
        format!(" {left}{separator}{right}"),
    );
    let documents =
        Documentation::load_from_str(ContentOrigin::TestEntityRust, &source, true, false);
    let (origin, chunks) = documents.iter().next().unwrap();
    assert_eq!(
        Vec::from_iter(chunks.iter().map(|chunk| chunk.as_str())),
        [expected_markdown.as_str()]
    );
    let suggestions = checkers.check(origin, chunks).unwrap();
    let context = format!("{detector}: {pattern}: {source}");
    assert_eq!(suggestions.len(), words.len(), "{context}");
    for (suggestion, word) in suggestions.iter().zip(words) {
        assert_eq!(suggestion.detector, detector, "{context}");
        assert_eq!(&suggestion.origin, origin, "{context}");
        assert_eq!(suggestion.checked_word.as_deref(), Some(word), "{context}");
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
        expected_markdown: &'static str,
        source: &'static str,
        patterns: [&'static str; 2],
        // Each pair is (typo, correction), with the captured word first.
        words: [(&'static str, &'static str); 2],
    }

    let cases = [
        PrecedenceCase {
            source: r#"/// 🦀 reciever->mesage
struct MessageRoute;"#,
            expected_markdown: r#" 🦀 reciever->mesage"#,
            patterns: ["(reciever)-", "reciever->"],
            words: [("reciever", "receiver"), ("mesage", "message")],
        },
        PrecedenceCase {
            source: r#"/// 🦉 reqeust->respnse
struct RequestRoute;"#,
            expected_markdown: r#" 🦉 reqeust->respnse"#,
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
                assert_eq!(
                    Vec::from_iter(chunks.iter().map(|chunk| chunk.as_str())),
                    [case.expected_markdown],
                    "{context}"
                );
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
                    assert_eq!(suggestion.detector, detector, "{context}");
                    assert_eq!(&suggestion.origin, origin, "{context}");
                    assert_eq!(suggestion.checked_word.as_deref(), Some(typo), "{context}");
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
        expected_markdown: &'static str,
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
            expected_markdown: r#" 🦀 CACHE::reciever->IGNROED->mesage::ID;occured"#,
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
            expected_markdown: r#" 🦉 succesful EVENT::conection,adress;lenght::END"#,
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
            expected_markdown: r#" 🐙 ROUTE::reqeust->respnse::END resutl"#,
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
            expected_markdown: r#" 🐢 TRACE::packte->timout::END warnign"#,
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
            expected_markdown: r#" 🦋 DOC::documnt->calback::END unkown"#,
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
            expected_markdown: r#" 🐝 LINK::mispelled->paramter::END commited"#,
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
            expected_markdown: r#" 🐬 PACKET::paylod->cheksum::END delivry"#,
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
            expected_markdown: r#" 🦎 BLOCK::threashold->notificaton::END interupt"#,
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
            expected_markdown: r#" 🦜 QUEUE::notifcation::END subscripton"#,
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
            expected_markdown: r#" 🦩 NEST::seperate::argumnt::END licnese"#,
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
            expected_markdown: r#" 🦦 NEST::reciept->attachement::END messsage"#,
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
            expected_markdown: r#" 🐌 WRAP::tikcet->reciever::END"#,
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
            expected_markdown: r#" 🐢 NEST::recievermesage::END"#,
            patterns: &[r"NEST::(?=(reciever))recieve(rmesage)::END"],
            expected_words: &[("reciever", "receiver")],
        },
        CompoundCase {
            name: "adjacent captures without a shared character retain both diagnostics",
            source: r#"/// 🦔 COMMAND::sucsessfailuer::END
struct CommandResult;"#,
            expected_markdown: r#" 🦔 COMMAND::sucsessfailuer::END"#,
            patterns: &[r"COMMAND::(sucsess)(failuer)::END"],
            expected_words: &[("sucsess", "success"), ("failuer", "failure")],
        },
        CompoundCase {
            name: "spanning captures may be recursively allow-listed as standalone fragments",
            source: r#"/// 🐳 SLOT::argumnt::licnese::END occurrance
struct LicensedArgument;"#,
            expected_markdown: r#" 🐳 SLOT::argumnt::licnese::END occurrance"#,
            patterns: &[r"SLOT::(argumnt::licnese)::END", r"argumnt::licnese"],
            expected_words: &[("occurrance", "occurrence")],
        },
        CompoundCase {
            name: "later per-fragment checking uses the boundaries left by compound matching",
            source: r#"/// 🦀 reciever->adres
struct ReceiverAddress;"#,
            expected_markdown: r#" 🦀 reciever->adres"#,
            patterns: &[r"reciever(?!-)", r"->(adres)"],
            expected_words: &[("adres", "address")],
        },
        CompoundCase {
            name: "cross-token matches cannot span whitespace-separated runs",
            source: r#"/// 🦊 whille :: untill
struct LoopBoundary;"#,
            expected_markdown: r#" 🦊 whille :: untill"#,
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
            assert_eq!(
                Vec::from_iter(chunks.iter().map(|chunk| chunk.as_str())),
                [case.expected_markdown],
                "{context}"
            );
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
                assert_eq!(suggestion.checked_word.as_deref(), Some(typo), "{context}");
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

#[cfg(all(feature = "hunspell", feature = "spellbook", feature = "zet"))]
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

#[test]
fn normalization_deduplicates_alternative_chunks_without_losing_checked_word() {
    let first = alternate_chunk("Unix", 10);
    let alternate = alternate_chunk("Other platform", 10);
    let distinct = alternate_chunk("Unix", 11);
    let mut original = shared_prose_suggestion(&first);
    original.checked_word = None;
    let duplicate = shared_prose_suggestion(&alternate);
    assert_ne!(original.range, duplicate.range);
    let expected = vec![duplicate.clone(), shared_prose_suggestion(&distinct)];
    let input = vec![original, expected[1].clone(), duplicate];

    for suggestions in [input.clone(), input.into_iter().rev().collect()] {
        let normalized = normalize_suggestions(suggestions);
        assert_eq!(normalized, expected);
        assert!(std::ptr::eq(normalized[0].chunk, &alternate));
        assert_eq!(normalized[0].checked_word.as_deref(), Some("typoo"));
    }
}

fn normalization_chunk() -> CheckableChunk {
    CheckableChunk::from_str(
        "abcdefghijklmnopqrstuvwxyz",
        indexmap::indexmap! { 0..26 => Span {
            start: LineColumn { line: 1, column: 0 },
            end: LineColumn { line: 1, column: 25 },
        } },
        crate::CommentVariant::TripleSlash,
    )
}

fn normalization_suggestion(
    chunk: &CheckableChunk,
    detector: Detector,
    range: Range,
) -> Suggestion<'_> {
    Suggestion {
        detector,
        origin: ContentOrigin::TestEntityRust,
        chunk,
        span: Span {
            start: LineColumn {
                line: 1,
                column: range.start,
            },
            end: LineColumn {
                line: 1,
                column: range.end - 1,
            },
        },
        range,
        checked_word: None,
        replacements: vec!["replacement".to_owned()],
        description: Some("description".to_owned()),
    }
}

#[test]
fn normalize_suggestions_orders_spans_then_detectors() {
    let chunk = normalization_chunk();
    let detectors = [Detector::ZSpell, Detector::Spellbook, Detector::NlpRules];
    for reverse in [false, true] {
        for offset in 0..detectors.len() {
            let mut input = detectors
                .iter()
                .cycle()
                .skip(offset)
                .take(detectors.len())
                .map(|&detector| normalization_suggestion(&chunk, detector, 5..10))
                .collect::<Vec<_>>();
            input.push(normalization_suggestion(&chunk, Detector::ZSpell, 15..20));
            input.push(normalization_suggestion(&chunk, Detector::ZSpell, 0..3));
            if reverse {
                input.reverse();
            }
            let normalized = normalize_suggestions(input);
            assert_eq!(
                normalized
                    .iter()
                    .map(|s| s.range.clone())
                    .collect::<Vec<_>>(),
                vec![0..3, 5..10, 15..20]
            );
            assert_eq!(normalized[1].detector, Detector::NlpRules);
        }
    }
}

#[test]
fn normalize_suggestions_prefers_checked_word_before_detector() {
    let chunk = normalization_chunk();
    for detector in [Detector::Hunspell, Detector::Spellbook, Detector::ZSpell] {
        let mut spelling = normalization_suggestion(&chunk, detector, 0..5);
        spelling.checked_word = Some("abcde".to_owned());
        let expected = spelling.clone();
        let input = vec![
            normalization_suggestion(&chunk, Detector::Dummy, 0..5),
            spelling,
            normalization_suggestion(&chunk, Detector::NlpRules, 0..5),
        ];
        assert_eq!(normalize_suggestions(input.clone()), vec![expected.clone()]);
        assert_eq!(
            normalize_suggestions(input.into_iter().rev().collect()),
            vec![expected]
        );
    }
}

#[test]
fn normalize_suggestions_compares_last_retained_spans() {
    let (expected_markdown, cases) = (
        "abcdefghijklmnopqrstuvwxyz",
        [
            // The third overlaps only the discarded second suggestion.
            (
                vec![0..5, 3..8, 6..11, 13..16],
                vec![("abcde", 0..5), ("ghijk", 6..11), ("nop", 13..16)],
            ),
            // The third overlaps the retained first, but not its predecessor.
            (
                vec![0..20, 2..5, 6..9],
                vec![("abcdefghijklmnopqrst", 0..20)],
            ),
            (
                vec![0..3, 5..8, 10..13],
                vec![("abc", 0..3), ("fgh", 5..8), ("klm", 10..13)],
            ),
            // Source span endpoints are inclusive, unlike chunk ranges.
            (vec![0..4, 3..7], vec![("abcd", 0..4)]),
        ],
    );
    let chunk = normalization_chunk();
    assert_eq!(chunk.as_str(), expected_markdown);

    for (ranges, expected) in cases {
        let input = ranges
            .into_iter()
            .rev()
            .map(|range| normalization_suggestion(&chunk, Detector::Dummy, range))
            .collect();
        let normalized = normalize_suggestions(input);
        assert_eq!(
            normalized
                .iter()
                .map(|suggestion| crate::sub_chars(
                    suggestion.chunk.as_str(),
                    suggestion.range.clone()
                ))
                .collect::<Vec<_>>(),
            expected.iter().map(|(text, _)| *text).collect::<Vec<_>>()
        );
        assert_eq!(
            normalized.into_iter().map(|s| s.range).collect::<Vec<_>>(),
            expected
                .into_iter()
                .map(|(_, range)| range)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn normalize_suggestions_preserves_unrelated_origins() {
    let chunk = normalization_chunk();
    let first = normalization_suggestion(&chunk, Detector::Dummy, 0..5);
    let mut second = normalization_suggestion(&chunk, Detector::Dummy, 2..7);
    second.origin = ContentOrigin::TestEntityCommonMark;
    let third = normalization_suggestion(&chunk, Detector::Dummy, 3..8);
    let input = vec![first, second, third];
    assert_eq!(normalize_suggestions(input.clone()), input);
}

#[test]
fn normalize_suggestions_empty_and_single() {
    assert!(normalize_suggestions(Vec::new()).is_empty());
    let chunk = normalization_chunk();
    let single = normalization_suggestion(&chunk, Detector::Dummy, 0..5);
    assert_eq!(normalize_suggestions(vec![single.clone()]), vec![single]);
}

#[cfg(all(feature = "hunspell", feature = "spellbook"))]
#[test]
fn mixed_spelling_backends_retain_hunspell_for_equal_spans() {
    let (source, expected_markdown, word) = (
        r#"/// Zzyzxqvocab Zzyzxqvocab
struct Example;"#,
        r#" Zzyzxqvocab Zzyzxqvocab"#,
        "Zzyzxqvocab",
    );
    let spelling = crate::config::HunspellConfig {
        skip_os_lookups: true,
        use_builtin: true,
        ..Default::default()
    };
    let checkers = Checkers::new(Config {
        hunspell: Some(spelling.clone()),
        spellbook: Some(spelling),
        zet: None,
        nlprules: None,
        ..Config::default()
    })
    .unwrap();
    let documents =
        Documentation::load_from_str(ContentOrigin::TestEntityRust, source, true, false);
    let (origin, chunks) = documents.iter().next().unwrap();
    assert_eq!(
        Vec::from_iter(chunks.iter().map(|chunk| chunk.as_str())),
        [expected_markdown]
    );
    let hunspell = checkers
        .hunspell
        .as_ref()
        .unwrap()
        .check(origin, chunks)
        .unwrap();
    let spellbook = checkers
        .spellbook
        .as_ref()
        .unwrap()
        .check(origin, chunks)
        .unwrap();
    assert_eq!(hunspell.len(), 2);
    assert_eq!(spellbook.len(), 2);
    for (hunspell, spellbook) in hunspell.iter().zip(&spellbook) {
        assert_eq!(hunspell.span, spellbook.span);
        assert_eq!(hunspell.checked_word.as_deref(), Some(word));
        assert_eq!(spellbook.checked_word.as_deref(), Some(word));
    }

    for _ in 0..8 {
        let suggestions = checkers.check(origin, chunks).unwrap();
        assert_eq!(suggestions.len(), 2);
        let mut spans = HashSet::new();
        for suggestion in suggestions {
            assert!(spans.insert(suggestion.span), "duplicate diagnostic span");
            assert_eq!(suggestion.detector, Detector::Hunspell);
            assert_eq!(suggestion.checked_word.as_deref(), Some(word));
        }
    }
}

#[cfg(feature = "nlprules")]
#[test]
fn grammar_suggestions_have_no_checked_word() {
    let (source, expected_markdown, unchecked_word) = (
        r#"/// This are wrong.
struct Example;"#,
        r#" This are wrong."#,
        "Zzyzxqvocab",
    );
    let checkers = Checkers::new(Config {
        hunspell: None,
        spellbook: None,
        zet: None,
        nlprules: Some(Default::default()),
        ..Config::default()
    })
    .unwrap();
    let documents =
        Documentation::load_from_str(ContentOrigin::TestEntityRust, source, true, false);
    let (origin, chunks) = documents.iter().next().unwrap();
    assert_eq!(
        Vec::from_iter(chunks.iter().map(|chunk| chunk.as_str())),
        [expected_markdown]
    );
    let suggestions = checkers.check(origin, chunks).unwrap();
    assert!(!suggestions.is_empty());
    assert!(suggestions
        .iter()
        .all(|s| s.detector == Detector::NlpRules && s.checked_word.is_none()));
    // Verification is dictionary membership, not a grammar or sentence check.
    checkers
        .verify_words(&std::collections::BTreeSet::from([
            unchecked_word.to_owned()
        ]))
        .unwrap();
}

#[cfg(any(feature = "hunspell", feature = "spellbook", feature = "zet"))]
mod dictionaries {
    use super::*;
    use crate::config::{HunspellConfig, Quirks, WrappedRegex};
    use fs_err as fs;
    use std::collections::BTreeSet;

    const WORD: &str = "Zzyzxqvocab";

    struct ExtraDictionary {
        directory: PathBuf,
        path: PathBuf,
        original: String,
    }

    impl ExtraDictionary {
        fn new() -> Self {
            let directory = std::env::temp_dir()
                .join(format!("cargo-spellcheck-reload-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&directory).unwrap();
            let path = directory.join("lingo.dic");
            fs::copy(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".config/lingo.dic"),
                &path,
            )
            .unwrap();
            let original = fs::read_to_string(&path).unwrap();
            Self {
                directory,
                path,
                original,
            }
        }

        fn add_word(&self) {
            let entries = self.original.lines().skip(1).collect::<Vec<_>>();
            fs::write(
                &self.path,
                format!("{}\n{}\n{WORD}\n", entries.len() + 1, entries.join("\n")),
            )
            .unwrap();
        }
    }

    impl Drop for ExtraDictionary {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    fn spelling_config(detectors: &[Detector], extra_dictionaries: Vec<PathBuf>) -> Config {
        let spelling = HunspellConfig {
            skip_os_lookups: true,
            use_builtin: true,
            extra_dictionaries,
            ..Default::default()
        };
        Config {
            hunspell: detectors
                .contains(&Detector::Hunspell)
                .then(|| spelling.clone()),
            spellbook: detectors
                .contains(&Detector::Spellbook)
                .then(|| spelling.clone()),
            zet: detectors.contains(&Detector::ZSpell).then_some(spelling),
            nlprules: None,
            ..Config::default()
        }
    }

    fn enabled_spelling_backends() -> Vec<Detector> {
        vec![
            #[cfg(feature = "hunspell")]
            Detector::Hunspell,
            #[cfg(feature = "spellbook")]
            Detector::Spellbook,
            #[cfg(feature = "zet")]
            Detector::ZSpell,
        ]
    }

    #[test]
    fn newly_registered_default_dictionary_is_loaded_only_on_reload() {
        let dictionary = ExtraDictionary::new();
        let detectors = enabled_spelling_backends();
        let mut config = spelling_config(&detectors, vec![]);
        assert_eq!(
            config
                .include_default_learning_dictionary(&dictionary.directory)
                .unwrap(),
            None
        );
        let mut checkers = Checkers::new(config).unwrap();
        assert!(matches!(
            checkers.word_membership(WORD).unwrap(),
            SpellcheckResult::Unrecognized(_)
        ));

        let path = Config::default_learning_dictionary(&dictionary.directory).unwrap();
        assert!(path.is_absolute());
        assert!(!path.exists());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let contents = format!("1\n{WORD}\n");
        fs::write(&path, &contents).unwrap();
        let reloader: &mut dyn DictionaryReload = &mut checkers;
        reloader.include_learning_dictionary(&path).unwrap();
        reloader.include_learning_dictionary(&path).unwrap();
        assert!(matches!(
            checkers.word_membership(WORD).unwrap(),
            SpellcheckResult::Unrecognized(_)
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), contents);
        for config in [
            checkers.config.hunspell.as_ref(),
            checkers.config.zet.as_ref(),
            checkers.config.spellbook.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            assert_eq!(config.extra_dictionaries, vec![path.clone()]);
            assert!(config.learning_dictionary.is_none());
        }

        checkers.reload_dictionaries().unwrap();
        assert_eq!(
            checkers.word_membership(WORD).unwrap(),
            SpellcheckResult::Recognized
        );
        for detector in detectors {
            let mut checker =
                Checkers::new(spelling_config(&[detector], vec![path.clone()])).unwrap();
            checker.reload_dictionaries().unwrap();
            assert_eq!(
                checker.word_membership(WORD).unwrap(),
                SpellcheckResult::Recognized,
                "{detector}"
            );
        }
    }

    fn reload_transactionally(detectors: &[Detector]) {
        let (source, expected_markdown, expected_metadata, words) = (
            r#"/// Zzyzxqvocab
struct Example;"#,
            r#" Zzyzxqvocab"#,
            vec![(
                Some(WORD.to_owned()),
                1..12,
                Span {
                    start: LineColumn { line: 1, column: 4 },
                    end: LineColumn {
                        line: 1,
                        column: 14,
                    },
                },
                *detectors
                    .iter()
                    .min_by_key(|detector| detector.as_str())
                    .unwrap(),
            )],
            BTreeSet::from(["Test".to_owned(), WORD.to_owned()]),
        );
        let metadata = |suggestions: Vec<Suggestion<'_>>| {
            Vec::from_iter(suggestions.into_iter().map(|suggestion| {
                (
                    suggestion.checked_word,
                    suggestion.range,
                    suggestion.span,
                    suggestion.detector,
                )
            }))
        };
        let dictionary = ExtraDictionary::new();
        let mut checkers =
            Checkers::new(spelling_config(detectors, vec![dictionary.path.clone()])).unwrap();
        let documents =
            Documentation::load_from_str(ContentOrigin::TestEntityRust, source, true, false);
        let (origin, chunks) = documents.iter().next().unwrap();
        assert_eq!(
            Vec::from_iter(chunks.iter().map(|chunk| chunk.as_str())),
            [expected_markdown]
        );
        assert_eq!(
            metadata(checkers.check(origin, chunks).unwrap()),
            expected_metadata
        );
        assert!(checkers.verify_words(&words).is_err());
        checkers.verify_words(&BTreeSet::new()).unwrap();

        dictionary.add_word();
        // Hunspell may read edited dictionaries lazily while generating replacements.
        // Recognition and diagnostic identity must still wait for an explicit reload.
        assert_eq!(
            metadata(checkers.check(origin, chunks).unwrap()),
            expected_metadata
        );
        assert!(checkers.verify_words(&words).is_err());
        let reloader: &mut dyn DictionaryReload = &mut checkers;
        reloader.reload_dictionaries().unwrap();
        reloader.verify_words(&words).unwrap();
        assert!(checkers.check(origin, chunks).unwrap().is_empty());

        #[cfg(feature = "hunspell")]
        let previous_hunspell = checkers.hunspell.clone();
        #[cfg(feature = "spellbook")]
        let previous_spellbook = checkers.spellbook.clone();
        #[cfg(feature = "zet")]
        let previous_zet = checkers.zet.clone();
        fs::remove_file(&dictionary.path).unwrap();
        assert!(checkers.reload_dictionaries().is_err());
        #[cfg(feature = "hunspell")]
        if let Some(previous) = previous_hunspell {
            assert!(std::sync::Arc::ptr_eq(
                &previous.0,
                &checkers.hunspell.as_ref().unwrap().0
            ));
        }
        #[cfg(feature = "spellbook")]
        if let Some(previous) = previous_spellbook {
            assert!(std::sync::Arc::ptr_eq(
                &previous.0,
                &checkers.spellbook.as_ref().unwrap().0
            ));
        }
        #[cfg(feature = "zet")]
        if let Some(previous) = previous_zet {
            assert!(std::sync::Arc::ptr_eq(
                &previous.0,
                &checkers.zet.as_ref().unwrap().0
            ));
        }
        // The previous dictionary, including its newly loaded word, survives failure.
        assert!(checkers.check(origin, chunks).unwrap().is_empty());

        fs::write(&dictionary.path, &dictionary.original).unwrap();
        assert!(checkers.check(origin, chunks).unwrap().is_empty());
        checkers.reload_dictionaries().unwrap();
        assert_eq!(
            metadata(checkers.check(origin, chunks).unwrap()),
            expected_metadata
        );
        assert!(checkers.verify_words(&words).is_err());
    }

    fn reload_learned_affixes_for_existing_root(detector: Detector) {
        let (base, affixes, empty, learned, source, expected_markdown, forms, expected_unknown) = (
            r#"2
Test
terminate/DS
"#,
            r#"SET UTF-8
PFX U Y 1
PFX U 0 un .
SFX D Y 2
SFX D 0 ed [^e]
SFX D 0 d e
SFX S Y 1
SFX S 0 s .
"#,
            r#"0
"#,
            r#"1
terminate/UD
"#,
            r#"/// Test terminate terminated terminates unterminate unterminated
struct Example;"#,
            r#" Test terminate terminated terminates unterminate unterminated"#,
            [
                ("Test", true),
                ("terminate", true),
                ("terminated", true),
                ("terminates", true),
                ("unterminate", false),
                ("unterminated", false),
            ],
            ["unterminate", "unterminated"],
        );
        let dictionary = ExtraDictionary::new();
        fs::write(dictionary.directory.join("en_US.dic"), base).unwrap();
        fs::write(dictionary.directory.join("en_US.aff"), affixes).unwrap();
        fs::write(&dictionary.path, empty).unwrap();
        let mut config = spelling_config(&[detector], vec![dictionary.path.clone()]);
        for spelling in [&mut config.hunspell, &mut config.spellbook, &mut config.zet]
            .into_iter()
            .flatten()
        {
            spelling.search_dirs = vec![dictionary.directory.clone()].into();
            spelling.use_builtin = false;
        }
        let mut checkers = Checkers::new(config).unwrap();
        for (word, recognized) in forms {
            assert_eq!(
                checkers.word_membership(word).unwrap(),
                if recognized {
                    SpellcheckResult::Recognized
                } else {
                    SpellcheckResult::Unrecognized(detector)
                },
                "{detector}: {word} before learning"
            );
        }
        let documents =
            Documentation::load_from_str(ContentOrigin::TestEntityRust, source, true, false);
        let (origin, chunks) = documents.iter().next().unwrap();
        assert_eq!(
            Vec::from_iter(chunks.iter().map(|chunk| chunk.as_str())),
            [expected_markdown]
        );
        let checked_tokens = |suggestions: Vec<Suggestion<'_>>| {
            Vec::from_iter(
                suggestions
                    .into_iter()
                    .map(|suggestion| suggestion.checked_word.unwrap()),
            )
        };
        assert_eq!(
            checked_tokens(checkers.check(origin, chunks).unwrap()),
            expected_unknown,
            "{detector}"
        );

        // Learning U must preserve the base-only S flag from terminate/DS.
        fs::write(&dictionary.path, learned).unwrap();
        assert_eq!(
            checkers.word_membership(expected_unknown[0]).unwrap(),
            SpellcheckResult::Unrecognized(detector),
            "{detector}"
        );
        assert_eq!(
            checked_tokens(checkers.check(origin, chunks).unwrap()),
            expected_unknown,
            "{detector}"
        );
        checkers.reload_dictionaries().unwrap();
        let words = BTreeSet::from(forms.map(|(word, _)| word.to_owned()));
        checkers
            .verify_words(&words)
            .unwrap_or_else(|error| panic!("{detector}: {error}"));
        for word in &words {
            assert_eq!(
                checkers.word_membership(word).unwrap(),
                SpellcheckResult::Recognized,
                "{detector}: {word} after reload"
            );
        }
        assert!(
            checkers.check(origin, chunks).unwrap().is_empty(),
            "{detector}"
        );
        assert_eq!(fs::read_to_string(&dictionary.path).unwrap(), learned);
    }

    #[cfg(feature = "hunspell")]
    #[test]
    fn hunspell_reloads_learned_affixes_for_existing_root() {
        reload_learned_affixes_for_existing_root(Detector::Hunspell);
    }

    #[cfg(feature = "spellbook")]
    #[test]
    fn spellbook_reloads_learned_affixes_for_existing_root() {
        reload_learned_affixes_for_existing_root(Detector::Spellbook);
    }

    #[cfg(feature = "zet")]
    #[test]
    fn zspell_reloads_learned_affixes_for_existing_root() {
        reload_learned_affixes_for_existing_root(Detector::ZSpell);
    }

    #[cfg(feature = "hunspell")]
    #[test]
    fn hunspell_builtin_dictionary_accepts_learned_unterminate() {
        let (empty, learned, original_forms, learned_forms) = (
            r#"0
"#,
            r#"1
terminate/UD
"#,
            [
                "terminate",
                "terminated",
                "terminates",
                "terminating",
                "termination",
                "terminations",
            ],
            ["unterminate", "unterminated"],
        );
        let dictionary = ExtraDictionary::new();
        fs::write(&dictionary.path, empty).unwrap();
        let mut config = spelling_config(&[Detector::Hunspell], vec![dictionary.path.clone()]);
        config.hunspell.as_mut().unwrap().search_dirs = vec![].into();
        let mut checkers = Checkers::new(config.clone()).unwrap();
        let original_forms = BTreeSet::from(original_forms.map(str::to_owned));
        let learned_forms = BTreeSet::from(learned_forms.map(str::to_owned));
        checkers.verify_words(&original_forms).unwrap();
        assert!(checkers.verify_words(&learned_forms).is_err());

        // The builtin terminate/DSGNX must not shadow this learned homonym.
        fs::write(&dictionary.path, learned).unwrap();
        assert!(checkers.verify_words(&learned_forms).is_err());
        checkers.reload_dictionaries().unwrap();
        checkers.verify_words(&learned_forms).unwrap();
        checkers.verify_words(&original_forms).unwrap();
        Checkers::new(config)
            .unwrap()
            .verify_words(&learned_forms)
            .unwrap();
        assert_eq!(fs::read_to_string(&dictionary.path).unwrap(), learned);
    }

    #[test]
    fn later_preparation_failure_never_installs_earlier_replacements() {
        let detectors = enabled_spelling_backends();
        for &failing in &detectors {
            let dictionary = ExtraDictionary::new();
            let mut checkers =
                Checkers::new(spelling_config(&detectors, vec![dictionary.path.clone()])).unwrap();
            dictionary.add_word();
            let config = match failing {
                Detector::Hunspell => checkers.config.hunspell.as_mut(),
                Detector::ZSpell => checkers.config.zet.as_mut(),
                Detector::Spellbook => checkers.config.spellbook.as_mut(),
                _ => unreachable!(),
            }
            .unwrap();
            config
                .extra_dictionaries
                .push(dictionary.directory.join("missing.dic"));
            assert!(checkers.reload_dictionaries().is_err(), "{failing}");

            // Even engines prepared before the failure must retain the old
            // dictionary. Checking only aggregate membership could hide a
            // partially installed replacement behind a later rejection.
            #[cfg(feature = "hunspell")]
            assert_eq!(
                checkers
                    .hunspell
                    .as_ref()
                    .unwrap()
                    .contains_word(WORD)
                    .unwrap(),
                SpellcheckResult::Unrecognized(Detector::Hunspell)
            );
            #[cfg(feature = "zet")]
            assert_eq!(
                checkers.zet.as_ref().unwrap().contains_word(WORD).unwrap(),
                SpellcheckResult::Unrecognized(Detector::ZSpell)
            );
            #[cfg(feature = "spellbook")]
            assert_eq!(
                checkers
                    .spellbook
                    .as_ref()
                    .unwrap()
                    .contains_word(WORD)
                    .unwrap(),
                SpellcheckResult::Unrecognized(Detector::Spellbook)
            );
        }
    }

    #[test]
    fn literal_membership_preserves_case_and_unicode() {
        let dictionary = ExtraDictionary::new();
        fs::write(&dictionary.path, "1\nZzyzxqvocabé\n").unwrap();
        for detector in enabled_spelling_backends() {
            let checkers =
                Checkers::new(spelling_config(&[detector], vec![dictionary.path.clone()])).unwrap();
            assert_eq!(
                checkers.word_membership("Zzyzxqvocabé").unwrap(),
                SpellcheckResult::Recognized,
                "{detector}"
            );
            for word in ["zzyzxqvocabé", "Zzyzxqvocabe", "Zzyzxqvocab"] {
                assert_eq!(
                    checkers.word_membership(word).unwrap(),
                    SpellcheckResult::Unrecognized(detector),
                    "{detector}: {word}"
                );
            }
        }
    }

    #[cfg(feature = "nlprules")]
    #[test]
    fn spelling_reload_leaves_live_grammar_untouched() {
        let (source, expected_markdown) = (
            r#"/// This are wrong. Zzyzxqvocab
struct Example;"#,
            r#" This are wrong. Zzyzxqvocab"#,
        );
        let dictionary = ExtraDictionary::new();
        let mut config =
            spelling_config(&enabled_spelling_backends(), vec![dictionary.path.clone()]);
        config.nlprules = Some(Default::default());
        let mut checkers = Checkers::new(config).unwrap();
        let documents =
            Documentation::load_from_str(ContentOrigin::TestEntityRust, source, true, false);
        let (origin, chunks) = documents.iter().next().unwrap();
        assert_eq!(
            Vec::from_iter(chunks.iter().map(|chunk| chunk.as_str())),
            [expected_markdown]
        );
        let grammar = checkers
            .nlprules
            .as_ref()
            .unwrap()
            .check(origin, chunks)
            .unwrap();
        assert!(!grammar.is_empty());

        // Reconstructing NLP would fail with this uncached, missing resource.
        checkers
            .config
            .nlprules
            .as_mut()
            .unwrap()
            .override_tokenizer = Some(dictionary.directory.join("missing-tokenizer.bin"));
        dictionary.add_word();
        checkers.reload_dictionaries().unwrap();
        assert_eq!(
            checkers.word_membership(WORD).unwrap(),
            SpellcheckResult::Recognized
        );
        assert_eq!(
            checkers
                .nlprules
                .as_ref()
                .unwrap()
                .check(origin, chunks)
                .unwrap(),
            grammar
        );
        fs::remove_file(&dictionary.path).unwrap();
        assert!(checkers.reload_dictionaries().is_err());
        assert_eq!(
            checkers
                .nlprules
                .as_ref()
                .unwrap()
                .check(origin, chunks)
                .unwrap(),
            grammar
        );
    }

    fn assert_rejection_names_word_and_backend(error: impl std::fmt::Display, detector: Detector) {
        let message = error.to_string();
        assert!(message.contains(WORD), "{message}");
        assert!(message.contains(&detector.to_string()), "{message}");
        assert!(message.contains("standalone"), "{message}");
    }

    fn verify_restricted_words(detector: Detector) {
        let dictionary = ExtraDictionary::new();
        let words = BTreeSet::from(["Test".to_owned(), WORD.to_owned()]);
        for (directive, flag) in [("ONLYINCOMPOUND", "C"), ("FORBIDDENWORD", "F")] {
            // zspell 0.5 does not enforce ONLYINCOMPOUND in check_word().
            // Verification must reflect the engine's actual membership semantics.
            if detector == Detector::ZSpell && directive == "ONLYINCOMPOUND" {
                continue;
            }
            fs::write(
                dictionary.directory.join("en_US.aff"),
                format!("SET UTF-8\n{directive} {flag}\n"),
            )
            .unwrap();
            fs::write(dictionary.directory.join("en_US.dic"), "1\nTest\n").unwrap();
            fs::write(&dictionary.path, format!("1\n{WORD}/{flag}\n")).unwrap();
            let mut config = spelling_config(&[detector], vec![dictionary.path.clone()]);
            for spelling in [&mut config.hunspell, &mut config.spellbook, &mut config.zet]
                .into_iter()
                .flatten()
            {
                spelling.search_dirs = vec![dictionary.directory.clone()].into();
                spelling.use_builtin = false;
            }
            let mut checkers = Checkers::new(config).unwrap();
            assert_rejection_names_word_and_backend(
                checkers.verify_words(&words).unwrap_err(),
                detector,
            );

            // Mirror adding a plain entry without removing the existing flagged entry.
            let persisted = format!("2\n{WORD}/{flag}\n{WORD}\n");
            fs::write(&dictionary.path, &persisted).unwrap();
            checkers.reload_dictionaries().unwrap();
            let verification = checkers.verify_words(&words);
            if directive == "ONLYINCOMPOUND" {
                verification.unwrap();
            } else if let Err(error) = verification {
                // A forbidden flag may still override the new plain entry. Report that
                // semantic failure without pretending to undo persistence or reload.
                assert_rejection_names_word_and_backend(error, detector);
            }
            assert_eq!(fs::read_to_string(&dictionary.path).unwrap(), persisted);
        }
    }

    #[cfg(feature = "hunspell")]
    #[test]
    fn hunspell_reload_is_transactional() {
        reload_transactionally(&[Detector::Hunspell]);
    }

    #[cfg(feature = "spellbook")]
    #[test]
    fn spellbook_reload_is_transactional() {
        reload_transactionally(&[Detector::Spellbook]);
    }

    #[cfg(feature = "zet")]
    #[test]
    fn zspell_reload_is_transactional() {
        reload_transactionally(&[Detector::ZSpell]);
    }

    #[test]
    fn all_spelling_dictionaries_reload_together() {
        reload_transactionally(&enabled_spelling_backends());
    }

    #[cfg(feature = "hunspell")]
    #[test]
    fn hunspell_verifies_standalone_word_restrictions() {
        verify_restricted_words(Detector::Hunspell);
    }

    #[cfg(feature = "spellbook")]
    #[test]
    fn spellbook_verifies_standalone_word_restrictions() {
        verify_restricted_words(Detector::Spellbook);
    }

    #[cfg(feature = "zet")]
    #[test]
    fn zspell_verifies_standalone_word_restrictions() {
        verify_restricted_words(Detector::ZSpell);
    }

    #[test]
    fn verification_checks_each_active_spelling_backend() {
        let dictionary = ExtraDictionary::new();
        dictionary.add_word();
        let detectors = enabled_spelling_backends();
        let words = BTreeSet::from(["Test".to_owned(), WORD.to_owned()]);
        for &rejecting in &detectors {
            let mut config = spelling_config(&detectors, vec![dictionary.path.clone()]);
            let spelling = match rejecting {
                Detector::Hunspell => config.hunspell.as_mut(),
                Detector::Spellbook => config.spellbook.as_mut(),
                Detector::ZSpell => config.zet.as_mut(),
                _ => unreachable!(),
            }
            .unwrap();
            spelling.extra_dictionaries.clear();
            let checkers = Checkers::new(config).unwrap();
            assert_eq!(
                checkers.word_membership("Test").unwrap(),
                SpellcheckResult::Recognized
            );
            assert_eq!(
                checkers.word_membership(WORD).unwrap(),
                SpellcheckResult::Unrecognized(rejecting)
            );
            assert_rejection_names_word_and_backend(
                checkers.verify_words(&words).unwrap_err(),
                rejecting,
            );
        }
    }

    #[test]
    fn spelling_metadata_precedes_markdown_mapping_and_transforms() {
        let cases = [
            (
                r#"/// Zzyzxqvocab
struct Example;"#,
                r#" Zzyzxqvocab"#,
                WORD,
                false,
            ),
            (
                r#"/// Zzyz*xqvocab*
struct Example;"#,
                r#" Zzyz*xqvocab*"#,
                WORD,
                true,
            ),
            (
                r#"/// prefixZzyzxqvocab
struct Example;"#,
                r#" prefixZzyzxqvocab"#,
                WORD,
                false,
            ),
            (
                r#"/// Zzyz*xqvocabé*
struct Example;"#,
                r#" Zzyz*xqvocabé*"#,
                "Zzyzxqvocabé",
                true,
            ),
            (
                r#"/// prefixzzyzxqvocabé
struct Example;"#,
                r#" prefixzzyzxqvocabé"#,
                "zzyzxqvocabé",
                false,
            ),
        ];
        for detector in enabled_spelling_backends() {
            let mut config = spelling_config(&[detector], vec![]);
            let quirks = Quirks {
                transform_regex: vec![WrappedRegex::from(
                    fancy_regex::Regex::new("^prefix(.+)$").unwrap(),
                )],
                ..Default::default()
            };
            for spelling in [&mut config.hunspell, &mut config.spellbook, &mut config.zet]
                .into_iter()
                .flatten()
            {
                spelling.quirks = quirks.clone();
            }
            let checkers = Checkers::new(config).unwrap();
            for (source, expected_markdown, checked_word, split_source) in cases {
                let documents = Documentation::load_from_str(
                    ContentOrigin::TestEntityRust,
                    source,
                    true,
                    false,
                );
                let (origin, chunks) = documents.iter().next().unwrap();
                assert_eq!(
                    Vec::from_iter(chunks.iter().map(|chunk| chunk.as_str())),
                    [expected_markdown],
                    "{source}"
                );
                let suggestions = checkers.check(origin, chunks).unwrap();
                assert!(!suggestions.is_empty(), "{detector:?}: {source}");
                for suggestion in &suggestions {
                    assert_eq!(suggestion.detector, detector);
                    assert_eq!(
                        suggestion.checked_word.as_deref(),
                        Some(checked_word),
                        "{source}"
                    );
                }
                if split_source {
                    assert!(suggestions
                        .iter()
                        .all(
                            |s| crate::sub_chars(s.chunk.as_str(), s.range.clone()) != checked_word
                        ));
                }
            }
        }
    }

    #[test]
    fn mixed_backends_retain_checked_word_metadata() {
        let (source, expected_markdown) = (
            r#"/// Zzyzxqvocab Zzyzxqvocab
struct Example;"#,
            r#" Zzyzxqvocab Zzyzxqvocab"#,
        );
        let detectors = enabled_spelling_backends();
        let expected_detector = *detectors.iter().min_by_key(|d| d.as_str()).unwrap();
        let checkers = Checkers::new(spelling_config(&detectors, vec![])).unwrap();
        let documents =
            Documentation::load_from_str(ContentOrigin::TestEntityRust, source, true, false);
        let (origin, chunks) = documents.iter().next().unwrap();
        assert_eq!(
            Vec::from_iter(chunks.iter().map(|chunk| chunk.as_str())),
            [expected_markdown]
        );
        for _ in 0..8 {
            let suggestions = checkers.check(origin, chunks).unwrap();
            assert_eq!(suggestions.len(), 2);
            for suggestion in suggestions {
                assert_eq!(suggestion.detector, expected_detector);
                assert_eq!(suggestion.checked_word.as_deref(), Some(WORD));
            }
        }
    }
}

#[cfg(any(feature = "hunspell", feature = "spellbook", feature = "zet"))]
#[test]
fn compound_capture_metadata_survives_markdown_mapping_and_normalization() {
    // Source fragments differ from the complete captures used for learning.
    let (source, expected_markdown, patterns, expected) = (
        r#"/// 🦀 WRAP::Zzyz*xqvocabé*->zzqx*notaword*::END mesage
struct Example;"#,
        r#" 🦀 WRAP::Zzyz*xqvocabé*->zzqx*notaword*::END mesage"#,
        [
            r"WRAP::((Zzyzxqvocabé)->(zzqxnotaword))::END",
            r"(Zzyzxqvocabé)->(zzqxnotaword)",
        ],
        [
            ("Zzyz", "Zzyzxqvocabé"),
            ("zzqx", "zzqxnotaword"),
            ("mesage", "mesage"),
        ],
    );
    let detectors = [
        #[cfg(feature = "hunspell")]
        Detector::Hunspell,
        #[cfg(feature = "spellbook")]
        Detector::Spellbook,
        #[cfg(feature = "zet")]
        Detector::ZSpell,
    ];
    let documents =
        Documentation::load_from_str(ContentOrigin::TestEntityRust, source, true, false);
    let (origin, chunks) = documents.iter().next().unwrap();
    assert_eq!(
        Vec::from_iter(chunks.iter().map(|chunk| chunk.as_str())),
        [expected_markdown]
    );
    for detector in detectors {
        let checkers = spelling_checkers_for_regex(detector, &patterns);
        let suggestions = checkers.check(origin, chunks).unwrap();
        let context = format!("{detector}: {source}");
        assert_eq!(
            suggestions.len(),
            expected.len(),
            "{context}: {suggestions:?}"
        );
        for (suggestion, (fragment, checked_word)) in suggestions.iter().zip(expected) {
            let chunk = suggestion.chunk.as_str();
            let start = chunk[..chunk.find(fragment).unwrap()].chars().count();
            let length = fragment.chars().count();
            let column = source[..source.find(fragment).unwrap()].chars().count();
            assert_eq!(suggestion.detector, detector, "{context}");
            assert_eq!(&suggestion.origin, origin, "{context}");
            assert_eq!(
                suggestion.checked_word.as_deref(),
                Some(checked_word),
                "{context}: {fragment}"
            );
            assert_eq!(suggestion.range, start..start + length, "{context}");
            assert_eq!(
                suggestion.span,
                Span {
                    start: LineColumn { line: 1, column },
                    end: LineColumn {
                        line: 1,
                        column: column + length - 1,
                    },
                },
                "{context}"
            );
            assert_eq!(
                crate::sub_chars(chunk, suggestion.range.clone()),
                fragment,
                "{context}"
            );
            assert_eq!(
                load_span_from(&mut source.as_bytes(), suggestion.span).unwrap(),
                fragment,
                "{context}"
            );
        }
    }
}

#[test]
fn no_active_spelling_engine_does_not_recognize_vocabulary() {
    let checkers = Checkers::new(Config {
        hunspell: None,
        zet: None,
        spellbook: None,
        nlprules: None,
        ..Config::default()
    })
    .unwrap();
    assert_eq!(
        checkers.word_membership("Test").unwrap(),
        SpellcheckResult::Unchecked
    );
    // Preserve the reload trait's no-op verification for grammar-only use.
    checkers
        .verify_words(&std::collections::BTreeSet::from(["Test".to_owned()]))
        .unwrap();
}
