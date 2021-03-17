//! Checker
//!
//! Trait to handle additional trackers.
//! Contains also helpers to avoid re-implementing generic
//! algorithms again and again, i.e. tokenization.

use crate::{Config, Detector, Documentation, Suggestion, SuggestionSet};

use anyhow::Result;

use crate::Range;
use log::debug;

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

    fn check<'a, 's>(docu: &'a Documentation, config: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's;
}

/// Returns absolute offsets and the data with the token in question.
///
/// Does not handle hyphenation yet or partial words at boundaries.
/// Returns the a vector of ranges for the input str.
///
/// All ranges are in characters.
fn tokenize_naive(s: &str, char_offset: usize, splitchars: &str) -> Vec<Range> {
    let mut started = false;
    // in characters
    let mut linear_start = 0;
    let mut linear_end;

    // very few sentences have more than 32 words, hence ranges.
    let mut bananasplit = Vec::with_capacity(32);

    let is_split_char = move |c: char| { c.is_whitespace() || splitchars.contains(c) };

    for (c_idx, (_byte_offset, c)) in s.char_indices().enumerate() {
        if is_split_char(c) {
            linear_end = c_idx;
            if started {
                let range = Range {
                    start: linear_start + char_offset,
                    end: linear_end + char_offset,
                };
                bananasplit.push(range);
            }
            started = false;
        } else {
            if !started {
                linear_start = c_idx;
                started = true;
            }
        }
    }
    // at the end of string, assume word complete
    // TODO for hypenation, check if line ends with a dash
    if started {
        if let Some((idx, _)) = s.char_indices().next_back() {
            // increase by one, since the range's end goes one beyond, end bounds is _exclusive_ for ranges
            let linear_end = idx + 1;
            bananasplit.push(linear_start..linear_end)
        } else {
            log::error!("BUG: Most likely lost a word when tokenizing!");
        }
    }
    bananasplit
}

/// Recommeneded default split chars for intra sentence spliting:
/// `splitchars = "\";:,?!#(){}[]\n\r/`"`
fn tokenize(s: &str, splitchars: &str) -> Result<Vec<Range>> {
    use std::{fs, str::FromStr};
    use srx::SRX;

    let srx = SRX::from_str(&fs::read_to_string("data/segment.srx")?)?;
    let english_rules = srx.language_rules("en_US");

    let previous_end = 0;
    let mut char_counter = previous_end;
    let mut acc = Vec::new();

    for byte_range in english_rules.split_ranges(s) {
        char_counter += s[previous_end..=(byte_range.start-1)].chars().count();
        acc.extend(tokenize_naive(&s[byte_range], char_counter, splitchars));
    }

    Ok(acc)
}

fn invoke_checker_inner<'a, 's, T>(
    documentation: &'a Documentation,
    config: Option<&T::Config>,
    collective: &mut SuggestionSet<'s>,
) -> Result<()>
where
    'a: 's,
    T: Checker,
{
    let config = config
        .as_ref()
        .expect("Must be Some(Config) if is_enabled returns true");

    let suggestions = T::check(documentation, *config)?;
    collective.join(suggestions);
    Ok(())
}

macro_rules! invoke_checker {
    ($feature:literal, $checker:ty, $documentation:ident, $config:expr, $config_inner:expr, $collective:expr) => {
        if !cfg!(feature = $feature) {
            debug!("Feature {} is disabled by compilation.", $feature);
        } else {
            #[cfg(feature = $feature)]
            {
                let detector = <$checker>::detector();
                let config = $config;
                if config.is_enabled(detector) {
                    debug!("Running {} checks.", detector);
                    invoke_checker_inner::<$checker>($documentation, $config_inner, $collective)?;
                } else {
                    debug!("Checker {} is disabled by configuration.", detector);
                }
            }
        }
    };
}

/// Check a full document for violations using the tools we have.
pub fn check<'a, 's>(documentation: &'a Documentation, config: &Config) -> Result<SuggestionSet<'s>>
where
    'a: 's,
{
    let mut collective = SuggestionSet::<'s>::new();

    invoke_checker!(
        "nlprules",
        self::nlprules::NlpRulesChecker,
        documentation,
        config,
        config.nlprules.as_ref(),
        &mut collective
    );

    invoke_checker!(
        "hunspell",
        self::hunspell::HunspellChecker,
        documentation,
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
    use crate::span::Span;
    use crate::ContentOrigin;
    use crate::LineColumn;

    use crate::fluff_up;

    const TEXT: &'static str = "With markdown removed, for sure.";
    lazy_static::lazy_static! {
        static ref TOKENS: Vec<&'static str> = vec![
            "With",
            "markdown",
            "removed",
            "for",
            "sure"
        ];
    }

    #[test]
    fn tokens() {
        let ranges: Vec<Range> = tokenize(TEXT, "{}()[]/|,.!?").unwrap();
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
                vec![format!("replacement_{}", index)]
            );
            assert_eq!(suggestion.span, *expected_span);
        }
    }

    #[test]
    fn extract_suggestions_simple() {
        const SIMPLE: &'static str = fluff_up!("two literals");

        /// keep in mind, `Span` bounds are inclusive, unlike Ranges, where range.end is _exclusive_
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

        /// keep in mind, `Span` bounds are inclusive, unlike Ranges, where range.end is _exclusive_
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

        /// keep in mind, `Span` bounds are inclusive, unlike Ranges, where range.end is _exclusive_
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
