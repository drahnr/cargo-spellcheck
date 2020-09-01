//! Reflow documentation comments to a desired line width.
//!
//! Note that for commonmark this might not be possible with links.
//! The reflow is done based on the comments no matter the content.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::checker::Checker;
use crate::documentation::{CheckableChunk, Documentation};
use crate::util::sub_chars;
use crate::{ContentOrigin, Detector, LineColumn, Range, Span, Suggestion, SuggestionSet, CommentVariant};

use indexmap::IndexMap;
use log::trace;
use pulldown_cmark::{Event, Options, Parser, Tag};

mod config;
pub use config::ReflowConfig;

mod iter;
pub use iter::{Gluon, Tokeneer};

#[derive(Debug)]
pub struct Reflow;

impl Checker for Reflow {
    type Config = ReflowConfig;
    fn check<'a, 's>(docu: &'a Documentation, config: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's,
    {
        unimplemented!("not yet")
    }
}

fn reflow_inner<'s>(
    origin: ContentOrigin,
    s: &'s str,
    range: Range,
    unbreakable_ranges: Vec<Range>,
    indentations: Vec<usize>,
    max_line_width: usize,
    variant: CommentVariant,
) -> Option<String> {
    let mut gluon = Gluon::new(s, range, max_line_width, indentations);
    gluon.add_unbreakables(unbreakable_ranges);
    let prefix = match variant {
        CommentVariant::CommonMark |
        CommentVariant::MacroDocEq => "",
        CommentVariant::TripleSlash => " ",
        CommentVariant::Unknown => return None,
    };

    let replacement = gluon.fold(String::new(), |mut acc, (_, content, _)| {
        acc.push_str(prefix);
        acc.push_str(&content);
        acc.push_str("\n");
        acc
    });

    // iterations above add a newline add the end, we have to remove it
    let replacement = replacement.trim_end_matches("\n").to_string();

    Some(replacement)
}

/// Reflow the documenation such that a maximum colomn constraint is met.
#[allow(unused)]
fn reflow<'s>(
    origin: ContentOrigin,
    chunk: &'s CheckableChunk,
    cfg: ReflowConfig,
) -> Vec<Suggestion<'s>> {
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

            let spans = chunk.find_spans_inclusive(range.clone());
            let span = Span {
                start: spans.first().unwrap().start,
                end: spans.last().unwrap().end,
            };

            // Get indentation per line as there is one span per line
            let indentations: Vec<usize> = spans.iter().map(|s| {
                s.start.column
            }).collect();

            if let Some(replacement) = reflow_inner(
                origin.clone(),
                chunk.as_str(),
                range.clone(),
                unbreakable_ranges.to_vec(),
                indentations,
                cfg.max_line_length,
                chunk.variant(),
            ) {
                acc.push(Suggestion {
                    chunk,
                    detector: Detector::Reflow,
                    origin: origin.clone(),
                    description: None,
                    range: range.clone(),
                    replacements: vec![replacement],
                    span: span.clone(),
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
                        // @todo the cover.end is exclusive and find_span() of CheckableChunk is inclusive, such that find_span()
                        // does not find the corresponding span, hence we have to subtract 1 here.
                        paragraph = store(cover.end - 1, unbreakables.as_slice());
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
    use crate::fluff_up;

    use crate::documentation::*;

    macro_rules! verify_reflow_inner {
        ([ $( $line:literal ),+ $(,)?] => $expected:literal) => {
            verify_reflow_inner!(80usize break [ $( $line ),+ ] => $expected );
        };
        ($n:literal break [ $( $line:literal ),+ $(,)?] => $expected:literal) => {
            const CONTENT: &'static str = fluff_up!($( $line ),+);
            let docs = Documentation::from((ContentOrigin::TestEntityRust, CONTENT));
            assert_eq!(docs.entry_count(), 1);
            let chunks = docs.get(&ContentOrigin::TestEntityRust).expect("Must contain dummy path");
            assert_eq!(dbg!(chunks).len(), 1);
            let chunk = &chunks[0];

            // @todo add tests with indentation and unbreakables or is this sufficiently covered by Gluon tests?
            let range = 0..CONTENT.len();
            let indentation: Vec<usize> = [0; 6].to_vec();
            let unbreakables = Vec::new();
            let replacement = reflow_inner(ContentOrigin::TestEntityRust, chunk.as_str(), range, unbreakables, indentation, $n, chunk.variant());

            assert!(replacement.is_some());
            assert_eq!(replacement.unwrap(), $expected);
            };
        ($line:literal => $expected:literal) => {
            verify_reflow_inner!([$line] => $expected);
        };
    }

    #[test]
    fn reflow_replacement_from_chunk() {
        verify_reflow_inner!(80 break ["This module contains documentation that \
is too long for one line and moreover, it \
spans over mulitple lines such that we can \
test our rewrapping algorithm.",
        "Smart, isn't it? Lorem ipsum and some more \
        blanket text without any meaning"] =>
        r#" This module contains documentation that is too long for one line and moreover,
 it spans over mulitple lines such that we can test our rewrapping algorithm.
 Smart, isn't it? Lorem ipsum and some more blanket text without any meaning"#);
    }

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
        let suggestion_set = reflow(ContentOrigin::TestEntityRust, chunk, cfg);
        let suggestions = suggestion_set
            .iter()
            .next()
            .expect("Must contain exactly one item");

            let replacement = suggestions.replacements.iter().next().expect("Must have a replacement");
            assert_eq!(replacement.as_str(), $expected);
        };
        ($line:literal => $expected:literal) => {
            reflow!([$line] => $expected);
        };
    }

    #[test]
    fn reflow_into_suggestion() {
        reflow!(44 break ["This module contains documentation thats \
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
 is too long for one line and moreover, it
 spans over mulitple lines such that we
 can test our rewrapping algorithm. Smart,
 isn't it? Lorem ipsum and some more
 blanket text without any meaning

 But lets also see what happens if there
 are two consecutive newlines in one
 connected documentation span."#);
    }

    #[test]
    fn reflow_shorter_than_limit() {
        reflow!(80 break ["This module contains documentation that is ok for one line"] =>
                r#" This module contains documentation that is ok for one line"#);
    }

    #[test]
    fn reflow_multiple_lines() {
        reflow!(43 break ["This module contains documentation that is broken",
                          "into multiple short lines resulting in multiple spans."] =>
                r#" This module contains documentation that
 is broken into multiple short lines
 resulting in multiple spans."#);
    }
    #[test]
    fn reflow_indentations() {
        const CONTENT: &'static str = r#"
    /// A comment with indentation that spans over
    /// two lines and should be rewrapped.
    struct Fluffy {};"#;

        const EXPECTED: &'static str = r#" A comment with indentation
 that spans over two lines
 and should be rewrapped."#;

        let docs = Documentation::from((ContentOrigin::TestEntityRust, CONTENT));
        assert_eq!(docs.entry_count(), 1);
        let chunks = docs
            .get(&ContentOrigin::TestEntityRust)
            .expect("Must contain dummy path");
        assert_eq!(dbg!(chunks).len(), 1);
        let chunk = &chunks[0];

        let cfg = ReflowConfig {
            max_line_length: 35,
            ..Default::default()
        };
        let suggestion_set = reflow(ContentOrigin::TestEntityRust, chunk, cfg);
        let suggestions = suggestion_set
            .iter()
            .next()
            .expect("Must contain exactly one item");

        let replacement = suggestions
            .replacements
            .iter()
            .next()
            .expect("Must have a replacement");
        assert_eq!(replacement.as_str(), EXPECTED);
    }

    #[test]
    fn reflow_chyrp() {
        const CONTENT: &'static str = r##"
    #[doc = r#"A comment with indentation that spans over
                two lines and should be rewrapped."#]
    struct Fluffy {};"##;

        const EXPECTED: &'static str = r#"A comment with indentation
that spans over two lines and
should be rewrapped."#;

        let docs = Documentation::from((ContentOrigin::TestEntityRust, CONTENT));
        assert_eq!(docs.entry_count(), 1);
        let chunks = docs
            .get(&ContentOrigin::TestEntityRust)
            .expect("Must contain dummy path");
        assert_eq!(dbg!(chunks).len(), 1);
        let chunk = &chunks[0];

        let cfg = ReflowConfig {
            max_line_length: 45,
            ..Default::default()
        };
        let suggestion_set = reflow(ContentOrigin::TestEntityRust, chunk, cfg);
        let suggestions = suggestion_set
            .iter()
            .next()
            .expect("Must contain exactly one item");

        let replacement = suggestions
            .replacements
            .iter()
            .next()
            .expect("Must have a replacement");
        assert_eq!(replacement.as_str(), EXPECTED);
    }
}
