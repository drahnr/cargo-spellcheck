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
        deduplicate_by_source(&mut suggestions);
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

/// Alternative chunks can report the same source text at different local ranges.
fn deduplicate_by_source(suggestions: &mut Vec<Suggestion<'_>>) {
    let mut seen = HashSet::new();
    suggestions.retain(|suggestion| {
        seen.insert((
            suggestion.origin.clone(),
            suggestion.detector,
            suggestion.span,
            suggestion.replacements.clone(),
            suggestion.description.clone(),
        ))
    });
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
