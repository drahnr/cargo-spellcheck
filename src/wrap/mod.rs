//! Wrap
//!
//! Re-wrap doc comments for prettier styling in code

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::checker::Checker;
use crate::Documentation;
use crate::{Detector, Suggestion, SuggestionSet};
use crate::Span;
use crate::LineColumn;

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
                    let mut new_lines = chunk
                        .as_str()
                        .split("\n\n")
                        .collect::<Vec<&str>>()
                        .iter()
                        .map(|s| s.replace("\n", "") )
                        .fold::<Vec<String>, _>(Vec::new(), |mut acc, comment| {
                            let mut new_comment = wrapper
                                .wrap_iter(comment.trim())
                                .map(|b| b.into_owned()).collect();
                            acc.append(&mut new_comment);
                            acc.push("".into());
                            acc
                        });
                    // remove last newline
                    let _ = new_lines.pop();
                    // @todo find proper span and range
                    let range = 0..chunk.as_str().len();
                    let span = Span { start: LineColumn { line: 0, column: 0}, end: LineColumn { line: 1, column: 5}};
                    acc.add(
                        origin.clone(),
                        Suggestion {
                            detector: Detector::Wrapper,
                            range,
                            span,
                            origin: origin.clone(),
                            replacements: vec!["".into()],
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
    fn rewrap() {
        let _ = env_logger::builder()
            .filter(None, log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        const TEST: &str = include_str!("../../demo/src/nested/just_very_long.rs");

        let stream =
            syn::parse_str::<proc_macro2::TokenStream>(TEST).expect("Must parse just fine");

        let d = Documentation::from((
            ContentOrigin::RustSourceFile(PathBuf::from("dummy/dummy.rs")),
            stream,
        ));

        let suggestions = Wrapper::check(&d, &WrapConfig::default()).expect("failed");
        dbg!(suggestions);

        assert!(false);
    }
}
