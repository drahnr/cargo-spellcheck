//! Checker
//!
//! Trait to handle additional trackers. Contains also helpers to avoid
//! re-implementing generic algorithms again and again, i.e. tokenization.

use crate::{CheckableChunk, Config, ContentOrigin, Detector, Suggestion};

use crate::errors::*;

mod cached;
use self::cached::Cached;

use std::collections::HashSet;

mod tokenize;

#[cfg(feature = "hunspell")]
pub(crate) use self::hunspell::HunspellChecker;
#[cfg(feature = "nlprules")]
pub(crate) use self::nlprules::NlpRulesChecker;
#[cfg(feature = "spellbook")]
pub(crate) use self::spellbook::SpellbookChecker;
pub(crate) use self::tokenize::*;
#[cfg(feature = "zet")]
pub(crate) use self::zspell::ZetChecker;

#[cfg(feature = "hunspell")]
mod hunspell;

#[cfg(feature = "zet")]
mod zspell;

#[cfg(feature = "spellbook")]
mod spellbook;

#[cfg(feature = "nlprules")]
mod nlprules;

mod dictaffix;

#[cfg(any(feature = "spellbook", feature = "zet", feature = "hunspell"))]
mod quirks;

#[cfg(any(feature = "spellbook", feature = "zet", feature = "hunspell"))]
struct SuggestionContext<'a, 's> {
    plain: &'a crate::documentation::PlainOverlay<'s>,
    chunk: &'s CheckableChunk,
    origin: &'a ContentOrigin,
    allow_concatenated: bool,
    allow_dashed: bool,
    allow_emojis: bool,
}

/// Implementation for a checker
pub trait Checker {
    type Config;

    fn detector() -> Detector;

    fn check<'a, 's>(
        &self,
        origin: &ContentOrigin,
        chunks: &'a [CheckableChunk],
    ) -> Result<Vec<Suggestion<'s>>>
    where
        'a: 's;
}

/// Check a full document for violations using the tools we have.
///
/// Only configured checkers are used.
pub struct Checkers {
    hunspell: Option<HunspellChecker>,
    #[cfg(feature = "zet")]
    zet: Option<ZetChecker>,
    #[cfg(feature = "spellbook")]
    spellbook: Option<SpellbookChecker>,
    nlprules: Option<NlpRulesChecker>,
}

impl Checkers {
    pub fn new(config: Config) -> Result<Self> {
        macro_rules! create_checker {
            ($feature:literal, $checker:ty, $config:expr, $checker_config:expr) => {
                if !cfg!(feature = $feature) {
                    log::debug!("Feature {} is disabled by compilation.", $feature);
                    None
                } else {
                    let config = $config;
                    let detector = <$checker>::detector();
                    if config.is_enabled(detector) {
                        log::debug!("Enabling {} checks.", detector);
                        Some(<$checker>::new($checker_config.unwrap())?)
                    } else {
                        log::debug!("Checker {detector} is disabled by configuration.");
                        None
                    }
                }
            };
        }

        let hunspell = create_checker!(
            "hunspell",
            HunspellChecker,
            &config,
            config.hunspell.as_ref()
        );
        #[cfg(feature = "zet")]
        let zet = create_checker!("zet", ZetChecker, &config, config.zet.as_ref());
        #[cfg(feature = "spellbook")]
        let spellbook = create_checker!(
            "spellbook",
            SpellbookChecker,
            &config,
            config.spellbook.as_ref()
        );
        let nlprules = create_checker!(
            "nlprules",
            NlpRulesChecker,
            &config,
            config.nlprules.as_ref()
        );
        Ok(Self {
            hunspell,
            #[cfg(feature = "zet")]
            zet,
            #[cfg(feature = "spellbook")]
            spellbook,
            nlprules,
        })
    }
}

impl Checker for Checkers {
    type Config = Config;

    fn detector() -> Detector {
        unreachable!()
    }

    fn check<'a, 's>(
        &self,
        origin: &ContentOrigin,
        chunks: &'a [CheckableChunk],
    ) -> Result<Vec<Suggestion<'s>>>
    where
        'a: 's,
    {
        let mut collective = HashSet::<Suggestion<'s>>::new();
        if let Some(ref hunspell) = self.hunspell {
            collective.extend(hunspell.check(origin, chunks)?);
        }
        #[cfg(feature = "zet")]
        if let Some(ref zet) = self.zet {
            collective.extend(zet.check(origin, chunks)?);
        }
        #[cfg(feature = "spellbook")]
        if let Some(ref spellbook) = self.spellbook {
            collective.extend(spellbook.check(origin, chunks)?);
        }
        if let Some(ref nlprule) = self.nlprules {
            collective.extend(nlprule.check(origin, chunks)?);
        }

        let mut suggestions: Vec<Suggestion<'s>> = Vec::from_iter(collective);
        suggestions.sort();
        if suggestions.is_empty() {
            return Ok(suggestions);
        }

        // Iterate through suggestions and identify overlapping ones.
        let suggestions = Vec::from_iter(suggestions.clone().into_iter().enumerate().filter_map(
            |(idx, cur)| {
                if idx == 0 || !cur.is_overlapped(&suggestions[idx - 1]) {
                    Some(cur)
                } else {
                    None
                }
            },
        ));

        Ok(suggestions)
    }
}

#[cfg(test)]
pub mod dummy;

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::load_span_from;
    use crate::ContentOrigin;
    use crate::Documentation;
    use crate::LineColumn;
    use crate::Range;
    use crate::Span;
    use std::path::PathBuf;

    use crate::fluff_up;

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
            let expected_extracts =
                load_span_from(&mut content.as_bytes(), *expected_span).unwrap();
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

    #[cfg(any(feature = "hunspell", feature = "spellbook", feature = "zet"))]
    fn spelling_checkers_for_regex(detector: Detector, pattern: Option<&str>) -> Checkers {
        use crate::config::{HunspellConfig, Quirks, WrappedRegex};

        let transform_regex = Vec::from_iter(
            pattern
                .into_iter()
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
                let checkers = spelling_checkers_for_regex(detector, Some(pattern));
                check_adjacent_spelling_cases(&checkers, detector, pattern, separators);
            }

            // An unrelated regex must not change existing plain-arrow diagnostics.
            let source = "/// Zzyzxqvocab->zzyzxqvocabé\nstruct Example;";
            let documents =
                Documentation::load_from_str(ContentOrigin::TestEntityRust, source, true, false);
            let (origin, chunks) = documents.iter().next().unwrap();
            let baseline = spelling_checkers_for_regex(detector, None)
                .check(origin, chunks)
                .unwrap();
            let unrelated = spelling_checkers_for_regex(detector, Some("::"))
                .check(origin, chunks)
                .unwrap();
            assert!(!baseline.is_empty(), "{detector}: {source}");
            assert_eq!(
                unrelated, baseline,
                "{detector}: unrelated :: regex: {source}"
            );
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
}
