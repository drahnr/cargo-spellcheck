//! Checker
//!
//! Trait to handle additional trackers. Contains also helpers to avoid
//! re-implementing generic algorithms again and again, i.e. tokenization.

use crate::{CheckableChunk, Config, ContentOrigin, Detector, Suggestion};

use crate::errors::*;

use log::debug;

mod tokenize;
pub(crate) use self::hunspell::HunspellChecker;
pub(crate) use self::nlprules::NlpRulesChecker;
pub(crate) use self::tokenize::*;

#[cfg(feature = "hunspell")]
mod hunspell;

#[cfg(feature = "nlprules")]
mod nlprules;

#[cfg(feature = "hunspell")]
mod quirks;

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
    nlprule: Option<NlpRulesChecker>,
}

impl Checkers {
    pub fn new(config: Config) -> Result<Self> {
        macro_rules! create_checker {
            ($feature:literal, $checker:ty, $config:expr, $checker_config:expr) => {
                if !cfg!(feature = $feature) {
                    debug!("Feature {} is disabled by compilation.", $feature);
                    None
                } else {
                    #[cfg(feature = $feature)]
                    {
                        let config = $config;
                        let detector = <$checker>::detector();
                        if config.is_enabled(detector) {
                            debug!("Enabling {} checks.", detector);
                            Some(<$checker>::new($checker_config.unwrap())?)
                        } else {
                            debug!("Checker {} is disabled by configuration.", detector);
                            None
                        }
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
        let nlprule = create_checker!(
            "nlprules",
            NlpRulesChecker,
            &config,
            config.nlprules.as_ref()
        );
        Ok(Self { hunspell, nlprule })
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
        let mut collective = Vec::<Suggestion<'s>>::with_capacity(chunks.len());
        if let Some(ref hunspell) = self.hunspell {
            collective.extend(hunspell.check(origin, chunks)?);
        }
        if let Some(ref nlprule) = self.nlprule {
            collective.extend(nlprule.check(origin, chunks)?);
        }

        collective.sort();

        Ok(collective)
    }
}

#[cfg(test)]
pub mod dummy;

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::load_span_from;
    use crate::span::Span;
    use crate::ContentOrigin;
    use crate::Documentation;
    use crate::LineColumn;
    use crate::Range;
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
        let dev_comments = false;
        let d = Documentation::load_from_str(ContentOrigin::TestEntityRust, content, dev_comments);
        let (origin, chunks) = d.into_iter().next().expect("Contains exactly one file");
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
                vec![format!("replacement_{}", index)],
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
}
