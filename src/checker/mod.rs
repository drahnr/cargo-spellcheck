//! Checker
//!
//! Trait to handle additional trackers.
//! Contains also helpers to avoid re-implementing generic
//! algorithms again and again, i.e. tokenization.

use crate::{
    CheckableChunk, Config, ContentOrigin, Detector, Documentation, Suggestion, SuggestionSet,
};

use crate::errors::*;

use log::debug;

mod tokenize;
pub(crate) use self::tokenize::*;

#[cfg(feature = "hunspell")]
mod hunspell;

#[cfg(feature = "nlprules")]
mod nlprules;

#[cfg(feature = "hunspell")]
mod quirks;

/// Implementation for a checker
pub(crate) trait Checker {
    type Config;

    fn detector() -> Detector;

    fn check<'a, 's>(
        origin: ContentOrigin,
        chunks: &'a [CheckableChunk],
        config: &'a Self::Config,
    ) -> Result<Vec<Suggestion<'s>>>
    where
        'a: 's;
}

fn invoke_checker_inner<'a, 's, T>(
    origin: ContentOrigin,
    chunks: &'a [CheckableChunk],
    config: Option<&'a T::Config>,
    collective: &mut Vec<Suggestion<'s>>,
) -> Result<()>
where
    'a: 's,
    CheckableChunk: 'a,
    T: Checker,
{
    let config = config
        .as_ref()
        .expect("Must be Some(Config) if is_enabled returns true");

    let suggestions = T::check(origin, chunks, *config)?;
    collective.extend(suggestions);
    Ok(())
}

macro_rules! invoke_checker {
    ($feature:literal, $checker:ty, $origin:expr, $chunks:expr, $config:expr, $checker_config:expr, $collective:expr) => {
        if !cfg!(feature = $feature) {
            debug!("Feature {} is disabled by compilation.", $feature);
        } else {
            #[cfg(feature = $feature)]
            {
                let config = $config;
                let detector = <$checker>::detector();
                if config.is_enabled(detector) {
                    debug!("Running {} checks.", detector);
                    invoke_checker_inner::<$checker>(
                        $origin,
                        $chunks,
                        $checker_config,
                        $collective,
                    )?;
                } else {
                    debug!("Checker {} is disabled by configuration.", detector);
                }
            }
        }
    };
}

/// Check a full document for violations using the tools we have.
pub fn check<'a, 's>(
    origin: ContentOrigin,
    chunks: &'a [CheckableChunk],
    config: &'a Config,
) -> Result<Vec<Suggestion<'s>>>
where
    'a: 's,
{
    let mut collective = Vec::<Suggestion<'s>>::with_capacity(chunks.len());

    invoke_checker!(
        "nlprules",
        self::nlprules::NlpRulesChecker,
        origin,
        chunks,
        config,
        config.nlprules.as_ref(),
        &mut collective
    );

    invoke_checker!(
        "hunspell",
        self::hunspell::HunspellChecker,
        origin,
        chunks,
        config,
        config.hunspell.as_ref(),
        &mut collective
    );

    collective.sort();

    Ok(collective)
}

#[cfg(test)]
pub mod dummy;

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::load_span_from;
    use crate::span::Span;
    use crate::ContentOrigin;
    use crate::LineColumn;
    use crate::Range;
    use std::path::PathBuf;

    use crate::fluff_up;

    const TEXT: &'static str = "With markdown removed, for sure.";
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
        let suggestion_set =
            dummy::DummyChecker::check(&d, &()).expect("Dummy extraction must never fail");

        // one file
        assert_eq!(suggestion_set.len(), 1);
        // with a known number of suggestions
        assert_eq!(suggestion_set.total_count(), expected_spans.len());
        let (_, suggestions) = suggestion_set
            .iter()
            .next()
            .expect("Must have valid 1st suggestion");

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
        const SIMPLE: &'static str = fluff_up!("two literals");

        /// keep in mind, `Span` bounds are inclusive, unlike Ranges, where `range.end` is _exclusive_
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
        const SIMPLE: &'static str = fluff_up!("two  literals ");

        /// keep in mind, `Span` bounds are inclusive, unlike Ranges, where `range.end` is _exclusive_
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
        const SIMPLE: &'static str = fluff_up!("  third  testcase ");

        /// keep in mind, `Span` bounds are inclusive, unlike Ranges, where `range.end` is _exclusive_
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
