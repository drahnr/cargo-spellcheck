//! Reflow documentation comments to a desired line width.
//!
//! Note that for commonmark this might not be possible with links.
//! The reflow is done based on the comments no matter the content.

use anyhow::Result;

use crate::checker::Checker;
use crate::documentation::{CheckableChunk, Documentation};

use crate::{CommentVariant, ContentOrigin, Detector, Range, Span, Suggestion, SuggestionSet};

use indexmap::IndexMap;

use pulldown_cmark::{Event, Options, Parser, Tag};

mod config;
pub use config::ReflowConfig;

mod iter;
pub use iter::{Gluon, Tokeneer};

/// Generate a string of whitespaces with length $n
macro_rules! whites {
    ($n:expr) => {{
        vec![" "; $n].join("")
    }};
}

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
                suggestions.extend(origin.clone(), reflow(origin, chunk, config)?);
            }
        }
        Ok(suggestions)
    }
}

/// Reflows a parsed commonmark paragraph contained in `s`
///
/// Returns the `Some(replacement)` string if a reflow has been performed and `None` otherwise.
///
/// `range` denotes the range of the paragraph of interest in the top-level `CheckableChunk`.
/// `unbreakable_ranges` contains all ranges of words/sequences which must not be split during
/// the reflow. They are relative to the top-level `CheckableChunk` similar to `range`. The indentation
/// vec contains the indentation for each line in `s`.
fn reflow_inner<'s>(
    s: &'s str,
    range: Range,
    unbreakable_ranges: &[Range],
    indentations: &[usize],
    max_line_width: usize,
    variant: CommentVariant,
) -> Option<String> {
    // make string and unbreakable ranges absolute
    let s_absolute = &s[range.clone()];
    let unbreakables = unbreakable_ranges
        .iter()
        .map(|r| (r.start - range.start)..(r.end - range.start));

    let mut gluon = Gluon::new(s_absolute, max_line_width, indentations);
    gluon.add_unbreakables(unbreakables);

    // vector of prefix for all lines
    let prefix: Vec<String> = match variant {
        CommentVariant::CommonMark | CommentVariant::MacroDocEq => {
            // first indent is 0
            let mut first: Vec<String> = vec!["".to_owned()];
            let pre: Vec<String> = indentations.iter().map(|i| whites!(*i)).collect();
            first.extend(pre);
            first
        }
        CommentVariant::TripleSlash => {
            // first indent is 1, afterwards we have origin indent + comment style
            let mut first: Vec<String> = vec![" ".to_owned()];
            let pre: Vec<String> = indentations
                .iter()
                .map(|i| whites!(i - 3) + "/// ")
                .collect();
            first.extend(pre);
            first
        }
        CommentVariant::DoubleSlashEM => {
            // first indent is 1, afterwards we have origin indent + comment style
            let mut first: Vec<String> = vec![" ".to_owned()];
            let pre: Vec<String> = indentations
                .iter()
                .map(|i| whites!(i - 3) + "//! ")
                .collect();
            first.extend(pre);
            first
        }
        CommentVariant::Unknown => return None,
    };

    let mut reflow_applied = false;

    let mut pre = prefix.iter();

    // construct replacement string from prefix and Gluon iterations
    let acc =
        gluon
            .zip(s_absolute.lines())
            .fold(String::new(), |mut acc, ((_, content, _), line)| {
                if line != &content {
                    reflow_applied = true;
                }
                // the current indentation, if there are more lines than before, we use the indent from the last one
                let pref = if let Some(p) = pre.next() {
                    p
                } else {
                    prefix.last().unwrap()
                };
                acc.push_str(pref);
                acc.push_str(&content);
                acc.push_str("\n");
                acc
            });

    // iterations above add a newline at the end, we have to remove it
    let replacement = acc.trim_end_matches("\n").to_string();

    if reflow_applied {
        Some(replacement)
    } else {
        None
    }
}

/// Collect reflowed Paragraphs in a Vector of suggestions
fn store_suggestion<'s>(
    acc: &mut Vec<Suggestion<'s>>,
    chunk: &'s CheckableChunk,
    origin: &ContentOrigin,
    paragraph: usize,
    end: usize,
    unbreakable_ranges: &[Range],
    max_line_width: usize,
) -> Result<usize> {
    let range = Range {
        start: paragraph,
        end,
    };
    let mut spans = chunk.find_covered_spans(range.clone());
    let span_start = if let Some(first) = spans.next() {
        first
    } else {
        return Ok(paragraph);
    };
    let span_end = if let Some(last) = spans.last() {
        last
    } else {
        span_start
    };
    let span = Span {
        start: span_start.start,
        end: span_end.end,
    };

    // Get indentation for each span, if a span covers multiple
    // lines, use same indentation for all lines
    let indentations = chunk
        .find_covered_spans(range.clone())
        .flat_map(|s| vec![s.start.column; s.end.line - s.start.line + 1])
        .collect::<Vec<usize>>();

    if let Some(replacement) = reflow_inner(
        chunk.as_str(),
        range.clone(),
        unbreakable_ranges,
        &indentations,
        max_line_width,
        chunk.variant(),
    ) {
        acc.push(Suggestion {
            chunk,
            detector: Detector::Reflow,
            origin: origin.clone(),
            description: None,
            range: range,
            replacements: vec![replacement],
            span: span,
        })
    }

    Ok(end) // a new beginning (maybe)
}

/// Parses a `CheckableChunk` and performs the rewrapping on contained paragraphs
fn reflow<'s>(
    origin: &ContentOrigin,
    chunk: &'s CheckableChunk,
    cfg: &ReflowConfig,
) -> Result<Vec<Suggestion<'s>>> {
    let parser = Parser::new_ext(chunk.as_str(), Options::all());

    let mut paragraph = 0_usize;
    let mut unbreakable_stack: Vec<Range> = Vec::with_capacity(16); // no more than 16 items will be nested, commonly it's 2 or 3
    let mut unbreakables = Vec::with_capacity(1024);

    let mut acc = Vec::with_capacity(256);

    for (event, cover) in parser.into_offset_iter() {
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
                        paragraph = store_suggestion(
                            &mut acc,
                            chunk,
                            origin,
                            paragraph,
                            paragraph,
                            unbreakable_stack.as_slice(),
                            cfg.max_line_length,
                        )?;
                        unbreakable_stack.clear();
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
                        paragraph = store_suggestion(
                            &mut acc,
                            chunk,
                            origin,
                            paragraph,
                            cover.end,
                            unbreakable_stack.as_slice(),
                            cfg.max_line_length,
                        )?;
                        unbreakable_stack.clear();
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
                paragraph = store_suggestion(
                    &mut acc,
                    chunk,
                    origin,
                    paragraph,
                    cover.end,
                    unbreakable_stack.as_slice(),
                    cfg.max_line_length,
                )?;
                unbreakable_stack.clear();
            }
            Event::Rule => {
                // paragraphs end before rules
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
    use crate::{chyrp_up, fluff_up};

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

            let range = 0..chunk.as_str().len();
            let indentation: Vec<usize> = [3; 6].to_vec();
            let unbreakables = Vec::new();
            let replacement = reflow_inner(
                chunk.as_str(),
                range,
                &unbreakables,
                &indentation,
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
test our rewrapping algorithm. With emojis: 🚤w🌴x🌋y🍈z🍉0",
        "Smart, isn't it? Lorem ipsum and some more \
        blanket text without any meaning"] =>
        r#" This module contains documentation that is too long for one line and
/// moreover, it spans over mulitple lines such that we can test our rewrapping
/// algorithm. With emojis: 🚤w🌴x🌋y🍈z🍉0 Smart, isn't it? Lorem ipsum and some more
/// blanket text without any meaning"#);
    }

    #[test]
    fn reflow_inner_not_required() {
        verify_reflow_inner!(80 break ["This module contains documentation."] =>
            r#" This module contains documentation."#);
        {
            verify_reflow_inner!(39 break ["This module contains documentation",
                "which is split in two lines"] =>
                r#" This module contains documentation
/// which is split in two lines"#);
        }
    }

    macro_rules! reflow {
        ([ $( $line:literal ),+ $(,)?] => $expected:literal, $no_reflow:expr) => {
            reflow!(80usize break [ $( $line ),+ ] => $expected, $no_reflow:expr);
        };
        ($n:literal break [ $( $line:literal ),+ $(,)?] => $expected:literal, $no_reflow:expr) => {
            const CONTENT:&'static str = fluff_up!($( $line ),+);
            let docs = Documentation::from((ContentOrigin::TestEntityRust, CONTENT));
            assert_eq!(docs.entry_count(), 1);
            let chunks = docs.get(&ContentOrigin::TestEntityRust).expect("Contains test data. qed");
            assert_eq!(dbg!(chunks).len(), 1);
            let chunk = &chunks[0];
            let _plain = chunk.erase_cmark();

            let cfg = ReflowConfig {
                max_line_length: $n,
            };
            let suggestion_set = reflow(&ContentOrigin::TestEntityRust, chunk, &cfg).expect("Reflow is working. qed");
            if $no_reflow {
                assert_eq!(suggestion_set.len(), 0);
            } else {
                let suggestions = suggestion_set
                    .iter()
                    .next()
                    .expect("Contains one suggestion. qed");

                    let replacement = suggestions.replacements.iter().next().expect("There exists a replacement. qed");
                    assert_eq!(replacement.as_str(), $expected);
            }
        };
        ($line:literal => $expected:literal, $no_reflow:expr) => {
            reflow!([$line] => $expected, $no_reflow:expr);
        };
    }

    macro_rules! reflow_chyrp {
        ([ $( $line:literal ),+ $(,)?] => $expected:literal, $no_reflow:expr) => {
            reflow_chyrp!(80usize break [ $( $line ),+ ] => $expected, $no_reflow:expr);
        };
        ($n:literal break [ $( $line:literal ),+ $(,)?] => $expected:literal, $no_reflow:expr) => {
            const CONTENT:&'static str = chyrp_up!($( $line ),+);
            let docs = Documentation::from((ContentOrigin::TestEntityRust, dbg!(CONTENT)));
            assert_eq!(docs.entry_count(), 1);
            let chunks = docs.get(&ContentOrigin::TestEntityRust).expect("Contains test data. qed");
            assert_eq!(dbg!(chunks).len(), 1);
            let chunk = &chunks[0];
            let _plain = chunk.erase_cmark();

            let cfg = ReflowConfig {
                max_line_length: $n,
            };
            let suggestion_set = reflow(&ContentOrigin::TestEntityRust, chunk, &cfg).expect("Reflow is working. qed");
            if $no_reflow {
                assert_eq!(suggestion_set.len(), 0);
            } else {
                let suggestions = suggestion_set
                    .iter()
                    .next()
                    .expect("Contains one suggestion. qed");

                    let replacement = suggestions.replacements.iter().next().expect("There exists a replacement. qed");
                    assert_eq!(replacement.as_str(), $expected);
            }
        };
        ($line:literal => $expected:literal, $no_reflow:expr) => {
            reflow_chyrp!([$line] => $expected, $no_reflow:expr);
        };
    }

    #[test]
    fn reflow_into_suggestion() {
        reflow!(44 break ["This module contains documentation thats \
is too long for one line and moreover, \
it spans over mulitple lines such that \
we can test our rewrapping algorithm. \
Smart, isn't it? Lorem ipsum and some more \
blanket text without any meaning.",
        "But lets also see what happens if \
there are two consecutive newlines \
in one connected documentation span."] =>

r#" This module contains documentation thats
/// is too long for one line and moreover, it
/// spans over mulitple lines such that we
/// can test our rewrapping algorithm. Smart,
/// isn't it? Lorem ipsum and some more
/// blanket text without any meaning. But
/// lets also see what happens if there are
/// two consecutive newlines in one connected
/// documentation span."#, false);
    }

    #[test]
    fn reflow_shorter_than_limit() {
        reflow!(80 break ["This module contains documentation that is ok for one line"] =>
                "", true);
    }

    #[test]
    fn reflow_multiple_lines() {
        reflow!(43 break ["This module contains documentation that is broken",
                          "into multiple short lines resulting in multiple spans."] =>
                r#" This module contains documentation that
/// is broken into multiple short lines
/// resulting in multiple spans."#, false);
    }
    #[test]
    fn reflow_indentations() {
        const CONTENT: &'static str = r#"
    /// A comment with indentation that spans over
    /// two lines and should be rewrapped.
    struct Fluffy {};"#;

        const EXPECTED: &'static str = r#" A comment with indentation
    /// that spans over two lines
    /// and should be rewrapped."#;

        let docs = Documentation::from((ContentOrigin::TestEntityRust, CONTENT));
        assert_eq!(docs.entry_count(), 1);
        let chunks = docs
            .get(&ContentOrigin::TestEntityRust)
            .expect("Contains test data. qed");
        assert_eq!(dbg!(chunks).len(), 1);
        let chunk = &chunks[0];

        let cfg = ReflowConfig {
            max_line_length: 35,
        };
        let suggestion_set =
            reflow(&ContentOrigin::TestEntityRust, chunk, &cfg).expect("Reflow is wokring. qed");

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
    fn reflow_doc_indentation() {
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
        };
        let suggestion_set =
            reflow(&ContentOrigin::TestEntityRust, chunk, &cfg).expect("Reflow is working. qed");

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
                          " `markdown` syntax which leads to __unbreakables__?  With emojis: 🚤w🌴x🌋y🍈z🍉0."] =>
            r#" Possible **ways** to run __rustc__ and request various
/// parts of LTO. `markdown` syntax which leads to
/// __unbreakables__? With emojis: 🚤w🌴x🌋y🍈z🍉0."#, false);
    }

    #[test]
    fn reflow_two_paragraphs_not_required() {
        reflow!(80 break ["A short paragraph followed by another one.", "", "Surprise, we have another parapgrah."]
                => "", true);
    }

    #[test]
    fn reflow_two_short_lines() {
        reflow!(70 break ["A short paragraph followed by two lines.", "Surprise, we have more lines here."]
                => " A short paragraph followed by two lines. Surprise, we have more
/// lines here.", false);
    }

    #[test]
    fn reflow_markdown_two_paragraphs() {
        const CONTENT: &'static str =
            "/// Possible __ways__ to run __rustc__ and request various parts of LTO.
///
/// Some more text after the newline which **represents** a paragraph";

        let expected = vec![
            r#" Possible __ways__ to run __rustc__ and request various
/// parts of LTO."#,
            r#" Some more text after the newline which **represents** a
/// paragraph"#,
        ];

        let _ = env_logger::Builder::new()
            .filter(None, log::LevelFilter::Debug)
            .is_test(true)
            .try_init();

        let docs = Documentation::from((ContentOrigin::TestEntityRust, CONTENT));
        assert_eq!(docs.entry_count(), 1);
        let chunks = docs
            .get(&ContentOrigin::TestEntityRust)
            .expect("Contains test data. qed");
        assert_eq!(dbg!(chunks).len(), 1);
        let chunk = &chunks[0];

        let cfg = ReflowConfig {
            max_line_length: 60,
        };

        let suggestion_set =
            reflow(&ContentOrigin::TestEntityRust, &chunk, &cfg).expect("Reflow is working. qed");

        for (sug, expected) in suggestion_set.iter().zip(expected) {
            assert_eq!(sug.replacements.len(), 1);
            let replacement = sug
                .replacements
                .iter()
                .next()
                .expect("An replacement exists. qed");

            assert_eq!(replacement.as_str(), expected);
        }
    }

    #[test]
    fn reflow_markdown_two_paragraphs_doc() {
        let chyrped = chyrp_up!(
            r#"A long comment that spans over two lines.

With a second part that is fine"#
        );

        let expected = vec![
            r#"A long comment that spans over two
         lines."#,
            r#"With a second part that is fine"#,
        ];

        let docs = Documentation::from((ContentOrigin::TestEntityRust, chyrped));
        assert_eq!(docs.entry_count(), 1);
        let chunks = docs
            .get(&ContentOrigin::TestEntityRust)
            .expect("Contains test data. qed");

        let cfg = ReflowConfig {
            max_line_length: 45,
        };

        for (chunk, expect) in chunks.iter().zip(expected) {
            let suggestion_set =
                reflow(&ContentOrigin::TestEntityRust, chunk, &cfg).expect("Reflow is working. qed");
            let sug = suggestion_set
                .iter()
                .next()
                .expect("Contains a suggestion. qed");
            let replacement = sug
                .replacements
                .iter()
                .next()
                .expect("An replacement exists. qed");
            assert_eq!(replacement.as_str(), expect);
        }
    }

    #[test]
    fn reflow_doc_short() {
        reflow_chyrp!(40 break ["a", "b", "c"] => r#"a b c"#, false);
    }

    #[test]
    fn reflow_doc_indent_middle() {
        reflow_chyrp!(28 break ["First line", "     Second line", "         third line"]
            => r#"First line Second
         line third line"#, false);
    }
}
