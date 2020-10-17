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
#[cfg(feature = "languagetool")]
mod languagetool;

#[cfg(any(feature = "languagetool", feature = "hunspell"))]
mod quirks;

/// Implementation for a checker
pub(crate) trait Checker {
    type Config;
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
fn tokenize(s: &str) -> Vec<Range> {
    let mut started = false;
    let mut linear_start = 0usize;
    let mut linear_end;
    let mut bananasplit = Vec::with_capacity(32);
    let _fin_char_idx = 0usize;

    let blacklist = "\";:,.?!#(){}[]-\n\r/`".to_owned();
    let is_ignore_char = |c: char| c.is_whitespace() || blacklist.contains(c);

    for (c_idx, (_byte_offset, c)) in s.char_indices().enumerate() {
        if is_ignore_char(c) {
            linear_end = c_idx;
            if started {
                bananasplit.push(linear_start..linear_end);
            }
            started = false;
        // TODO handle hyphenation
        // if c == '\n' {
        //     column = 0;
        //     line += 1;
        // }
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

/// Check a full document for violations using the tools we have.
pub fn check<'a, 's>(documentation: &'a Documentation, config: &Config) -> Result<SuggestionSet<'s>>
where
    'a: 's,
{
    let mut collective = SuggestionSet::<'s>::new();

    #[cfg(feature = "languagetool")]
    {
        if config.is_enabled(Detector::LanguageTool) {
            debug!("Running LanguageTool checks");
            let config = config
                .languagetool
                .as_ref()
                .expect("Must be Some(LanguageToolConfig) if is_enabled returns true");

            let mut suggestions =
                self::languagetool::LanguageToolChecker::check(documentation, config)?;
            collective.join(suggestions);
        }
    }

    #[cfg(feature = "hunspell")]
    {
        if config.is_enabled(Detector::Hunspell) {
            debug!("Running Hunspell checks");
            let config = config
                .hunspell
                .as_ref()
                .expect("Must be Some(HunspellConfig) if is_enabled returns true");
            let suggestions = self::hunspell::HunspellChecker::check(documentation, config)?;
            collective.join(suggestions);
        }
    }

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
        let ranges: Vec<Range> = tokenize(TEXT);
        for (range, expect) in ranges.into_iter().zip(TOKENS.iter()) {
            assert_eq!(&&TEXT[range], expect);
        }
    }

    pub fn extraction_test_body(content: &str, expected_spans: &[Span]) {
        let _ = env_logger::builder()
            .filter(None, log::LevelFilter::Trace)
            .is_test(true)
            .try_init();
        let d = Documentation::from((ContentOrigin::TestEntityRust, content));
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
