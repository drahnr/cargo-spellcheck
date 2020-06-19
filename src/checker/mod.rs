use crate::{Config, Detector, Documentation, Suggestion, SuggestionSet};

use anyhow::Result;

use crate::Range;
use log::debug;

#[cfg(feature = "hunspell")]
mod hunspell;
#[cfg(feature = "languagetool")]
mod languagetool;

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
fn tokenize(s: &str) -> Vec<Range> {
    let mut started = false;
    let mut linear_start = 0usize;
    let mut linear_end;
    let mut bananasplit = Vec::with_capacity(32);
    let _fin_char_idx = 0usize;

    let blacklist = "\";:,.?!#(){}[]-\n\r/`".to_owned();
    let is_ignore_char = |c: char| c.is_whitespace() || blacklist.contains(c);

    for (c_idx, c) in s.char_indices() {
        if is_ignore_char(c) {
            linear_end = c_idx;
            if started {
                bananasplit.push(linear_start..linear_end);
            }
            started = false;
        // @todo handle hyphenation
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
    // @todo for hypenation, check if line ends with a dash
    if started {
        if let Some((idx, _)) = s.char_indices().next_back() {
            // increase by one, since the range's end goes one beyond
            let linear_end = idx + 1;
            bananasplit.push(linear_start..linear_end)
        } else {
            log::warn!("Most liekly lost a word when tokenizing! BUG");
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
            if let Ok(mut suggestions) =
                self::languagetool::LanguageToolChecker::check(documentation, config)
            {
                collective.join(suggestions);
            }
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
            if let Ok(suggestions) = self::hunspell::HunspellChecker::check(documentation, config) {
                collective.join(suggestions);
            }
        }
    }

    Ok(collective)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use proc_macro2::{LineColumn, Literal};
    use std::path::PathBuf;

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

    pub struct TestChecker;

    impl Checker for TestChecker {
        type Config = ();

        fn check<'a, 's>(docu: &'a Documentation, _: &Self::Config) -> Result<SuggestionSet<'s>>
        where
            'a: 's,
        {
            let suggestions = docu.iter().try_fold::<SuggestionSet, _, Result<_>>(
                SuggestionSet::new(),
                |mut acc, (path, literal_sets)| {
                    let plain = literal_sets.iter().next().unwrap().erase_markdown();
                    for range in tokenize(plain.as_str()) {
                        let detector = Detector::Hunspell;
                        for (literal, span) in plain.linear_range_to_spans(range.clone()) {
                            debug!("TestChecker: Span    {:?}", &span);
                            debug!("TestChecker: literal {:?}", &literal);
                            let replacements = vec!["literal".to_string(); 1];
                            let suggestion = Suggestion {
                                detector,
                                span: span,
                                path: PathBuf::from(path),
                                replacements,
                                literal: literal.into(),
                                description: None,
                            };
                            acc.add(PathBuf::from(path), suggestion);
                        }
                    }
                    Ok(acc)
                },
            )?;

            Ok(suggestions)
        }
    }

    #[test]
    fn test_checker() {
        let mut d = Documentation::new();
        let dummy_path = PathBuf::from("dummy/dummy.rs");
        d.append_literal(&dummy_path, Literal::string("literal"));
        let c = TestChecker::check(&d, &()).expect("TestChecker failed");

        assert_eq!(c.len(), 1);
        for (path, suggestions) in c {
            assert_eq!(path, dummy_path);
            let s = suggestions.iter().next().unwrap();
            assert_eq!(
                s.span,
                crate::span::Span {
                    start: LineColumn { line: 1, column: 1 },
                    end: LineColumn { line: 1, column: 7 },
                }
            );
            assert_eq!(s.replacements, vec!["literal".to_string(); 1]);
        }
    }
}
