//! Reflow documentation comments to a desired line width.
//!
//! Note that for commonmark this might not be possible with links.
//! The reflow is done based on the comments no matter the content.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::checker::Checker;
use crate::documentation::{CheckableChunk, Documentation};
use crate::util::sub_chars;
use crate::{ContentOrigin, Detector, LineColumn, Range, Span, Suggestion, SuggestionSet};

use indexmap::IndexMap;
use log::trace;
use pulldown_cmark::{Event, Options, Parser, Tag};


mod config;
pub use config::ReflowConfig;

mod iter;
pub use iter::Warp;

#[derive(Debug)]
pub struct Reflow;

impl Checker for Reflow {
    type Config = ReflowConfig;
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
                            detector: Detector::Reflow,
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

fn reflow_inner<'s>(
    origin: ContentOrigin,
    s: &'s str,
    range: Range,
    unbreakable_ranges: &[Range],
) -> Option<String> {
    let mut warper = unimplemented!();
    unimplemented!()
}

/// Reflow the documenation such that a maximum colomn constraint is met.
#[allow(unused)]
fn reflow<'s>(origin: ContentOrigin, chunk: &'s CheckableChunk) -> Vec<Suggestion<'s>> {
    let parser = Parser::new_ext(chunk.as_str(), Options::all());

    let mut paragraph = 0usize;
    let mut unbreakable_stack: Vec<Range> = Vec::with_capacity(16); // no more than 16 items will be nested, commonly it's 2 or 3
    let mut unbreakables = Vec::with_capacity(1024);

    let mut acc = Vec::with_capacity(256);

    for (event, cover) in parser.into_offset_iter() {
        let mut store = |end: usize, unbreakable_ranges: &[Range]| -> usize {
            let range = Range {
                start: paragraph,
                end,
            };
            if let Some(replacement) = reflow_inner(
                origin.clone(),
                chunk.as_str(),
                range.clone(),
                unbreakables.as_slice(),
            ) {
                acc.push(Suggestion {
                    chunk,
                    detector: Detector::Reflow,
                    origin: origin.clone(),
                    description: None,
                    range: range.clone(),
                    replacements: vec![replacement],
                    span: unimplemented!("Obtain the span"),
                })
            }

            end // a new beginning (maybe)
        };

        match event {
            Event::Start(tag) => {
                // @todo check links
                match tag {
                    Tag::Image(_, _, _)
                    | Tag::Link(_, _, _)
                    | Tag::Strong
                    | Tag::Emphasis
                    | Tag::Strikethrough => {
                        unbreakable_stack.push(cover);
                    }
                    Tag::Paragraph => {
                        paragraph = cover.start;
                    }
                    _ => {
                        // all of these break a reflow-able chunk
                        paragraph = store(paragraph, unbreakables.as_slice());
                    }
                }
            }
            Event::End(tag) => {
                match tag {
                    Tag::Image(_, _, _)
                    | Tag::Link(_, _, _)
                    | Tag::Strong
                    | Tag::Emphasis
                    | Tag::Strikethrough => {
                        // technically we only need the bottom-most range, since all others - by def - are contained in there
                        // so there
                        if unbreakable_stack.len() == 1 {
                            unbreakables.push(cover);
                        } else if let Some(parent) = unbreakables.last() {
                            debug_assert!(parent.contains(&cover.start));
                            debug_assert!(parent.contains(&(cover.end - 1)));
                        }
                    }
                    Tag::Paragraph => {
                        // regular end of paragraph
                        paragraph = store(cover.end, unbreakables.as_slice());
                    }
                    _ => {
                        paragraph = cover.end;
                    }
                }
            }
            Event::Text(_s) => {}
            Event::Code(_s) => {}
            Event::Html(_s) => {
                // @todo verify this does not interfere with paragraphs
            }
            Event::FootnoteReference(_s) => {
                // boring
            }
            Event::SoftBreak => {
                // ignored
            }
            Event::HardBreak => {
                paragraph = store(cover.end, unbreakables.as_slice());
            }
            Event::Rule => {
                // @todo how to proceed to past this? do all paragraphs end before
                paragraph = store(cover.end, unbreakables.as_slice());
            }
            Event::TaskListMarker(_b) => {
                // ignored
            }
        }
    }

    acc
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
            let _plain = chunk.erase_cmark();

            let cfg = ReflowConfig {
                max_line_length: $n,
                .. Default::default()
            };
        let suggestion_set = Reflow::check(&docs, &cfg)
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
