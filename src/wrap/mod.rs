//! Re-wrap documentation comments to a desired line width.
//!
//! Will handle hyphenation eventually if desired.

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
pub struct Wrapper;

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
    use crate::end2end;
    use crate::fluff_up;

    use crate::documentation::*;
    use std::path::PathBuf;

    macro_rules! reflow {
        ([ $( $line:literal ),+ $(,)?] => $expected:literal) => {
            reflow!(80usize break [ $( $line ),+ ] => $expected );
        };
        ($n:literal break [ $( $line:literal ),+ $(,)?] => $expected:literal) => {
            const CONTENT:&'static str = fluff_up!($( $line ),+);
            let docs = Documentation::from((ContentOrigin::TestEntityRust, CONTENT));
            assert_eq!(docs.entry_count(), 1);
            let chunks = docs.get(&ContentOrigin::TestEntityRust).expect("Must contain dummy path");
            assert_eq!(dbg!(chunks).len(), 1);
            let chunk = &chunks[0];
            let _plain = chunk.erase_markdown();

            let cfg = WrapConfig {
                max_line_length: $n,
                .. Default::default()
            };
        let suggestion_set = Wrapper::check(&docs, &cfg)
            .expect("Must not fail to extract suggestions");
        let (_, suggestions) = suggestion_set
            .iter()
            .next()
            .expect("Must contain exactly one item");

            let suggestion = suggestions.into_iter().next().expect("Missing");
            let replacement = suggestion.replacements.iter().next().expect("Must have a replacement");
            assert_eq!(replacement.as_str(), $expected);
        };
        ($line:literal => $expected:literal) => {
            reflow!([$line] => $expected);
        };
    }

    #[test]
    fn rewrap_into_suggestion() {
        reflow!(41 break ["This module contains documentation thats \
is too long for one line and moreover, \
it spans over mulitple lines such that \
we can test our rewrapping algorithm. \
Smart, isn't it? Lorem ipsum and some more \
blanket text without any meaning",
        "",
        "But lets also see what happens if \
there are two consecutive newlines \
in one connected documentation span."] =>

r#" This module contains documentation thats
 is too long for one line and moreover,
 it spans over mulitple lines such that
 we can test our rewrapping algorithm.
 Smart, isn't it? Lorem ipsum and some
 more blanket text without any meaning

 But lets also see what happens if there
 are two consecutive newlines in one
 connected documentation span."#);
    }
}
