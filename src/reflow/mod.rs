//! Reflow documentation comments to a desired line width.
//!
//! Note that for commonmark this might not be possible with links.
//! The reflow is done based on the comments no matter the content.

use anyhow::Result;

use crate::checker::Checker;
use crate::documentation::{CheckableChunk, Documentation};

use crate::{
    CommentVariant, ContentOrigin, Detector, LineColumn, Range, Span, Suggestion, SuggestionSet,
};

use indexmap::IndexMap;

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
        let mut suggestions = SuggestionSet::new();
        for (origin, chunks) in docu.iter() {
            for chunk in chunks {
                suggestions.extend(origin.clone(), reflow(origin.clone(), chunk, config)?);
            }
        }
        Ok(suggestions)
    }
}

fn reflow_inner<'s>(
    _origin: ContentOrigin,
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
    if replacement == s {
        None
    } else {
        Some(replacement)
    }
}

/// Reflow the documenation such that a maximum colomn constraint is met.
#[allow(unused)]
fn reflow<'s>(
    origin: ContentOrigin,
    chunk: &'s CheckableChunk,
    cfg: &ReflowConfig,
) -> Result<Vec<Suggestion<'s>>> {
    let parser = Parser::new_ext(chunk.as_str(), Options::all());

    let mut paragraph = 0usize;
    let mut unbreakable_stack: Vec<Range> = Vec::with_capacity(16); // no more than 16 items will be nested, commonly it's 2 or 3
    let mut unbreakables = Vec::with_capacity(1024);

    let mut acc = Vec::with_capacity(256);

    for (event, cover) in parser.into_offset_iter() {
        let mut store = |end: usize, unbreakable_ranges: &[Range]| -> Result<usize> {
            let range = Range {
                start: paragraph,
                end,
            };

            let mut spans = chunk.find_covered_spans(range.clone());

            // debug_assert!(!spans.is_empty());

            let span_start: LineColumn = if let Some(first) = spans.next() {
                first.start
            } else {
                // anyhow::bail!("Missing spans");
                return Ok(paragraph);
            };
            let span_end: LineColumn = if let Some(last) = spans.last() {
                last.end
            } else {
                span_start
            };

            let span = Span {
                start: span_start,
                end: span_end,
            };

            // Get indentation for each span, if a span covers multiple
            // lines, use same indentation for all lines
            let indentations = chunk
                .find_covered_spans(range.clone())
                .map(|s| vec![s.start.column; s.end.line - s.start.line + 1])
                .flatten()
                .collect::<Vec<usize>>();

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

            Ok(end) // a new beginning (maybe)
        };

        match event {
            Event::Start(tag) => {
                // TODO check links
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
                        paragraph = store(paragraph, unbreakables.as_slice())?;
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
                        } else if let Some(parent) = unbreakable_stack.last() {
                            debug_assert!(parent.contains(&cover.start));
                            debug_assert!(parent.contains(&(cover.end - 1)));
                        }
                    }
                    Tag::Paragraph => {
                        // regular end of paragraph
                        paragraph = store(cover.end, unbreakables.as_slice())?;
                    }
                    _ => {
                        paragraph = cover.end;
                    }
                }
            }
            Event::Text(_s) => {}
            Event::Code(_s) => {}
            Event::Html(_s) => {
                // TODO verify this does not interfere with paragraphs
            }
            Event::FootnoteReference(_s) => {
                // boring
            }
            Event::SoftBreak => {
                // ignored
            }
            Event::HardBreak => {
                paragraph = store(cover.end, unbreakables.as_slice())?;
            }
            Event::Rule => {
                // TODO how to proceed to past this? do all paragraphs end before
                paragraph = store(cover.end, unbreakables.as_slice())?;
            }
            Event::TaskListMarker(_b) => {
                // ignored
            }
        }
    }

    Ok(acc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fluff_up;

    use crate::documentation::*;

    macro_rules! verify_reflow_inner {
        ([ $( $line:literal ),+ $(,)?] => $expected:literal) => {
            verify_reflow_inner!(80usize break [ $( $line ),+ ] => $expected);
        };
        ($n:literal break [ $( $line:literal ),+ $(,)?] => $expected:literal) => {
            const CONTENT: &'static str = fluff_up!($( $line ),+);
            let docs = Documentation::from((ContentOrigin::TestEntityRust, CONTENT));
            assert_eq!(docs.entry_count(), 1);
            let chunks = docs.get(&ContentOrigin::TestEntityRust).expect("Must contain dummy path");
            assert_eq!(dbg!(chunks).len(), 1);
            let chunk = &chunks[0];

            // TODO add tests with indentation and unbreakables or is this sufficiently covered by Gluon tests?
            let range = 0..CONTENT.len();
            let indentation: Vec<usize> = [0; 6].to_vec();
            let unbreakables = Vec::new();
            let replacement = reflow_inner(ContentOrigin::TestEntityRust,
                chunk.as_str(),
                range,
                unbreakables,
                indentation,
                $n,
                chunk.variant()
            );

            if let Some(repl) = replacement {
                assert_eq!(repl, $expected);
            } else {
                for line in CONTENT.lines() {
                    assert!(line.len() < $n);
                }
            }
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

    #[test]
    fn reflow_inner_not_required() {
        verify_reflow_inner!(80 break ["This module contains documentation."] =>
            r#" This module contains documentation."#);
        {
            verify_reflow_inner!(39 break ["This module contains documentation",
                "which is split two lines"] =>
                r#" This module contains documentation
 which is split two lines"#);
        }
    }

    macro_rules! reflow {
        ([ $( $line:literal ),+ $(,)?] => $expected:literal) => {
            reflow!(80usize break [ $( $line ),+ ] => $expected );
        };
        ($n:literal break [ $( $line:literal ),+ $(,)?] => $expected:literal) => {
            const CONTENT:&'static str = fluff_up!($( $line ),+);
            let docs = Documentation::from((ContentOrigin::TestEntityRust, CONTENT));
            assert_eq!(docs.entry_count(), 1);
            let chunks = docs.get(&ContentOrigin::TestEntityRust).expect("Contains test data. qed");
            assert_eq!(dbg!(chunks).len(), 1);
            let chunk = &chunks[0];
            let _plain = chunk.erase_cmark();

            let cfg = ReflowConfig {
                max_line_length: $n,
                .. Default::default()
            };
        let suggestion_set = reflow(ContentOrigin::TestEntityRust, chunk, &cfg).expect("Reflow is working. qed");
        let suggestions = suggestion_set
            .iter()
            .next()
            .expect("Contains one suggestion. qed");

            let replacement = suggestions.replacements.iter().next().expect("There exists a replacement. qed");
            assert_eq!(replacement.as_str(), $expected);
        };
        ($line:literal => $expected:literal) => {
            reflow!([$line] => $expected).expect("Reflow does not error. qed");
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
            .expect("Contains test data. qed");
        assert_eq!(dbg!(chunks).len(), 1);
        let chunk = &chunks[0];

        let cfg = ReflowConfig {
            max_line_length: 35,
            ..Default::default()
        };
        let suggestion_set = reflow(ContentOrigin::TestEntityRust, chunk, &cfg)
            .expect("Reflow is wokring. qed");

        let suggestions = suggestion_set
            .iter()
            .next()
            .expect("Contains one suggestion. qed");

        let replacement = suggestions
            .replacements
            .iter()
            .next()
            .expect("There is a replacement. qed");
        assert_eq!(replacement.as_str(), EXPECTED);
    }

    #[test]
    fn reflow_chyrp() {
        const CONTENT: &'static str = r##"
    #[doc = r#"A comment with indentation that spans over
                two lines and should be rewrapped.
            "#]
    struct Fluffy {};"##;

        const EXPECTED: &'static str = r#"A comment with indentation
that spans over two lines and
should be rewrapped."#;

        let docs = Documentation::from((ContentOrigin::TestEntityRust, CONTENT));
        assert_eq!(docs.entry_count(), 1);
        let chunks = docs
            .get(&ContentOrigin::TestEntityRust)
            .expect("Contains test data. qed");
        assert_eq!(dbg!(chunks).len(), 1);
        let chunk = &chunks[0];

        let cfg = ReflowConfig {
            max_line_length: 45,
            ..Default::default()
        };
        let suggestion_set = reflow(ContentOrigin::TestEntityRust, chunk, &cfg)
            .expect("Reflow is working. qed");

        let suggestions = suggestion_set
            .iter()
            .next()
            .expect("Contains one suggestion. qed");

        let replacement = suggestions
            .replacements
            .iter()
            .next()
            .expect("There is a replacement. qed");
        assert_eq!(replacement.as_str(), EXPECTED);
    }

    #[test]
    fn reflow_markdown() {
        reflow!(60 break ["Possible **ways** to run __rustc__ and request various parts of LTO.",
                          " `markdown` syntax which leads to __unbreakables__? "] =>
            r#" Possible **ways** to run __rustc__ and request various
 parts of LTO. `markdown` syntax which leads to
 __unbreakables__?"#);
    }
}
