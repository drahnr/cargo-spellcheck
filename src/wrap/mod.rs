//! Wrap
//!
//! Re-wrap doc comments for prettier styling in code

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::checker::Checker;
use crate::Documentation;
use crate::LineColumn;
use crate::Span;
use crate::{Detector, Suggestion, SuggestionSet};

/// Parameters for wrapping doc comments
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapConfig {
    /// Hard limit for absolute length of lines
    max_line_length: usize,
}

impl Default for WrapConfig {
    fn default() -> Self {
        WrapConfig {
            max_line_length: 70,
        }
    }
}

#[derive(Debug)]
pub struct Wrapper {}

impl Checker for Wrapper {
    type Config = WrapConfig;
    fn check<'a, 's>(docu: &'a Documentation, config: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's,
    {
        let wrapper = textwrap::Wrapper::new(config.max_line_length)
            .subsequent_indent(" ")
            .initial_indent(" ");
        let suggestions = docu.iter().try_fold::<SuggestionSet, _, Result<_>>(
            SuggestionSet::new(),
            |mut acc, (origin, chunks)| {
                for chunk in chunks {
                    let mut new_lines = dbg!(&chunk)
                        .as_str()
                        .split("\n\n")
                        .collect::<Vec<&str>>()
                        .iter()
                        .map(|s| s.replace("\n", ""))
                        .fold::<Vec<String>, _>(Vec::new(), |mut acc, comment| {
                            let mut new_comment = wrapper
                                .wrap_iter(comment.trim())
                                .map(|b| b.into_owned())
                                .collect();
                            acc.append(&mut new_comment);
                            acc.push("".into());
                            acc
                        });
                    // remove last newline
                    let _ = new_lines.pop();
                    // @todo that's too easy
                    if new_lines.len() == chunk.as_str().lines().count() {
                        log::trace!("No rewrapping required for '{:?}'", chunk);
                        break; // the chunk did not change
                    }
                    // @todo find proper span and range
                    let range = dbg!(0..chunk.as_str().len());
                    let mut start = 1000..1001;
                    let mut end = 0..1;
                    let mut span = Span {
                        start: LineColumn { line: 0, column: 0 },
                        end: LineColumn { line: 0, column: 0 },
                    };
                    chunk.iter().for_each(|(r, s)| {
                        if start.start > r.start {
                            start = r.clone();
                            span.start = s.start;
                        }
                        if end.end < r.end {
                            end = r.clone();
                            span.end = s.end;
                        }
                    });
                    dbg!(&span);

                    acc.add(
                        origin.clone(),
                        Suggestion {
                            detector: Detector::Wrapper,
                            range,
                            span,
                            origin: origin.clone(),
                            replacements: vec![new_lines.join("\n")],
                            chunk,
                            description: Some("Rewrapped".to_owned()),
                        },
                    )
                }
                Ok(acc)
            },
        )?;

        Ok(suggestions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::*;
    use std::path::PathBuf;

    #[test]
    fn rewrap_into_suggestion() {
        let _ = env_logger::builder()
            .filter(None, log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        const TEST: &str = include_str!("../../demo/src/nested/just_very_long.rs");
        const TEST_STR: &str = r#"This module contains documentation thats is too long for one line and moreover, it spans over mulitple lines such that we can test our rewrapping algorithm. Smart, isn't it? Lorem ipsum and some more blanket text without any meaning

But lets also see what happens if there are two consecutive newlines in one connected documentation span."#;

        let stream =
            syn::parse_str::<proc_macro2::TokenStream>(TEST).expect("Must parse just fine");

        let d = Documentation::from((
            ContentOrigin::RustSourceFile(PathBuf::from("dummy/dummy.rs")),
            stream,
        ));

        let wrapped = textwrap::Wrapper::new(WrapConfig::default().max_line_length)
            .initial_indent(" ")
            .subsequent_indent(" ").fill(TEST_STR);
        // the string resulting from fill() has one whitespace in the empty line which
        let wrapped = wrapped.replace("\n \n", "\n\n");

        let suggestions = Wrapper::check(&d, &WrapConfig::default()).expect("failed");

        // one file
        assert_eq!(suggestions.len(), 1);
        // one too long comment
        assert_eq!(suggestions.total_count(), 1);
        for (orig, suggestion_vec) in suggestions {
            for suggestion in suggestion_vec {
                assert_eq!(suggestion.replacements.first().unwrap(), &wrapped);
            }
        }
    }
}
