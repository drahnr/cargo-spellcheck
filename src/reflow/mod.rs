//! Reflow documentation comments to a desired line width.
//!
//! Note that for commonmark this might not be possible with links.
//! The reflow is done based on the comments no matter the content.

use anyhow::{anyhow, Result};

use crate::checker::Checker;
use crate::documentation::{CheckableChunk, Documentation};

use crate::{CommentVariant, ContentOrigin, Detector, Range, Span, Suggestion, SuggestionSet};

use indexmap::IndexMap;

use pulldown_cmark::{Event, Options, Parser, Tag};

mod config;
pub use config::ReflowConfig;

mod iter;
pub use iter::{Gluon, Tokeneer};

use rayon::prelude::*;

#[derive(Debug)]
pub struct Reflow;

impl Checker for Reflow {
    type Config = ReflowConfig;
    fn check<'a, 's>(docu: &'a Documentation, config: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's,
    {
        let suggestions = docu
            .par_iter()
            .try_fold::<SuggestionSet, Result<SuggestionSet>, _, _>(
                || SuggestionSet::new(),
                |mut acc, (origin, chunks)| {
                    for chunk in chunks {
                        let suggestions = reflow(origin, chunk, config)?;
                        acc.extend(origin.clone(), suggestions);
                    }
                    Ok(acc)
                },
            )
            .try_reduce(
                || SuggestionSet::new(),
                |mut a, b| {
                    a.join(b);
                    Ok(a)
                },
            )?;
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
    variant: &CommentVariant,
) -> Result<Option<String>> {
    // make string and unbreakable ranges absolute
    let s_absolute = crate::util::sub_chars(s, range.clone());
    let unbreakables = unbreakable_ranges
        .iter()
        .map(|r| (r.start.saturating_sub(range.start))..(r.end.saturating_sub(range.start)));

    let mut gluon = Gluon::new(&s_absolute, max_line_width, &indentations);
    gluon.add_unbreakables(unbreakables);

    let mut reflow_applied = false;
    let mut lines = s_absolute.lines();
    let mut indents = indentations.iter();
    let last_indent = indentations
        .last()
        .ok_or_else(|| anyhow!("No line indentation present."))?
        .saturating_sub(variant.prefix_len());
    // First line has to be without indent and variant prefix.
    // If there is nothing to reflow, just pretend there was no reflow
    let (_, content, _) = match gluon.next() {
        Some(c) => c,
        None => return Ok(None),
    };
    if lines.next() != Some(&content) {
        reflow_applied = true;
    }
    let acc = content + &variant.suffix_string() + "\n";

    // construct replacement string from prefix and Gluon iterations
    let content = gluon.fold(acc, |mut acc, (_, content, _)| {
        if lines.next() == Some(&content) {
            reflow_applied = true;
        }
        let pre = if let Some(i) = indents.next() {
            " ".repeat((*i).saturating_sub(variant.prefix_len()))
        } else {
            " ".repeat(last_indent)
        };

        acc.push_str(&pre);
        acc.push_str(&variant.prefix_string());
        acc.push_str(&content);
        acc.push_str(&variant.suffix_string());
        acc.push_str("\n");
        acc
    });

    // remove last new line
    let content = if let Some(c) = content.strip_suffix("\n") {
        c.to_string()
    } else {
        return Ok(None);
    };

    Ok(if reflow_applied {
        // for MacroDocEq comments, we also have to remove the last closing delimiter
        let content = content
            .strip_suffix(&variant.suffix_string())
            .map(|content| content.to_owned())
            .unwrap_or_else(|| content);
        Some(content)
    } else {
        None
    })
}

/// Collect reflowed Paragraphs in a Vector of suggestions
fn store_suggestion<'s>(
    chunk: &'s CheckableChunk,
    origin: &ContentOrigin,
    paragraph: usize,
    end: usize,
    unbreakable_ranges: &[Range],
    max_line_width: usize,
) -> Result<(usize, Option<Suggestion<'s>>)> {
    let range = Range {
        start: paragraph,
        end,
    };
    #[cfg(debug_assertions)]
    let s = chunk.as_str();
    #[cfg(debug_assertions)]
    let sb = s.as_bytes();

    #[cfg(debug_assertions)]
    log::trace!(
        "reflow::store_suggestion[range]: {}",
        crate::util::sub_chars(s, range.clone())
    );

    let spans = chunk.find_spans(range.clone());
    let mut spans = spans.into_iter().map(|(_range, span)| span);
    let span_start = if let Some(first) = spans.next() {
        first
    } else {
        return Ok((paragraph, None));
    };
    let span_end = if let Some(last) = spans.last() {
        last
    } else {
        span_start
    };
    let mut span = Span {
        start: span_start.start,
        end: span_end.end,
    };
    #[cfg(debug_assertions)]
    log::trace!(
        "reflow::store_suggestion[span]: {}",
        crate::util::load_span_from(sb, span).unwrap()
    );

    // TODO: find_covered_spans() seems to report a span which is off by one for TrppleSlash comments. Ultimately,
    // the problem is somewhere inside chunk's source_mapping?!
    if chunk.variant() == CommentVariant::TripleSlash
        || chunk.variant() == CommentVariant::DoubleSlashEM
    {
        span.start.column += 1;
    }

    // Get indentation for each span, if a span covers multiple
    // lines, use same indentation for all lines
    let indentations = chunk
        .find_covered_spans(range.clone())
        .flat_map(|s| {
            debug_assert!(s.start.line <= s.end.line);
            vec![s.start.column; s.end.line.saturating_sub(s.start.line) + 1]
        })
        .collect::<Vec<usize>>();

    Ok((
        end,
        reflow_inner(
            chunk.as_str(),
            range.clone(),
            unbreakable_ranges,
            &indentations,
            max_line_width,
            &chunk.variant(),
        )?
        .map(|replacement| {
            let suggestion = Suggestion {
                chunk,
                detector: Detector::Reflow,
                origin: origin.clone(),
                description: None,
                range,
                replacements: vec![replacement],
                span,
            };
            suggestion
        }),
    ))
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
                        let (p, suggestion) = store_suggestion(
                            chunk,
                            origin,
                            paragraph,
                            paragraph,
                            unbreakable_stack.as_slice(),
                            cfg.max_line_length,
                        )?;
                        paragraph = p;
                        if let Some(suggestion) = suggestion {
                            acc.push(suggestion);
                        }
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
                        let (p, suggestion) = store_suggestion(
                            chunk,
                            origin,
                            paragraph,
                            cover.end,
                            unbreakable_stack.as_slice(),
                            cfg.max_line_length,
                        )?;
                        paragraph = p;
                        if let Some(suggestion) = suggestion {
                            acc.push(suggestion);
                        }
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
                let (p, suggestion) = store_suggestion(
                    chunk,
                    origin,
                    paragraph,
                    cover.end,
                    unbreakable_stack.as_slice(),
                    cfg.max_line_length,
                )?;
                paragraph = p;
                if let Some(suggestion) = suggestion {
                    acc.push(suggestion);
                }
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
    use crate::{LineColumn, Span};

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
                &chunk.variant()
            );

            if let Ok(Some(repl)) = replacement {
                // TODO: check indentation
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
        r#"This module contains documentation that is too long for one line and
/// moreover, it spans over mulitple lines such that we can test our rewrapping
/// algorithm. With emojis: 🚤w🌴x🌋y🍈z🍉0 Smart, isn't it? Lorem ipsum and some more
/// blanket text without any meaning"#);
    }

    #[test]
    fn reflow_inner_not_required() {
        verify_reflow_inner!(80 break ["This module contains documentation."] =>
            r#"This module contains documentation."#);
        {
            verify_reflow_inner!(39 break ["This module contains documentation",
                "which is split in two lines"] =>
                r#"This module contains documentation
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
            println!("{}", CONTENT);

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
                    println!("#####{}", replacement);
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

r#"This module contains documentation thats
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
                r#"This module contains documentation that
/// is broken into multiple short lines
/// resulting in multiple spans."#, false);
    }
    #[test]
    fn reflow_indentations() {
        const CONTENT: &'static str = r#"
    /// A comment with indentation that spans over
    /// two lines and should be rewrapped.
    struct Fluffy {};"#;

        const EXPECTED: &'static str = r#"A comment with indentation
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

        const EXPECTED: &'static str = r##"A comment with indentation"#]
    #[doc = r#"that spans over two lines and"#]
    #[doc = r#"should be rewrapped."##;

        let docs = Documentation::from((ContentOrigin::TestEntityRust, CONTENT));
        assert_eq!(dbg!(&docs).entry_count(), 1);
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
            r#"Possible **ways** to run __rustc__ and request various
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
                => "A short paragraph followed by two lines. Surprise, we have more
/// lines here.", false);
    }

    #[test]
    fn reflow_markdown_two_paragraphs() {
        const CONTENT: &'static str =
            "/// Possible __ways__ to run __rustc__ and request various parts of LTO.
///
/// Some more text after the newline which **represents** a paragraph";

        let expected = vec![
            r#"Possible __ways__ to run __rustc__ and request various
/// parts of LTO."#,
            r#"Some more text after the newline which **represents** a
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
        println!("{}", chyrped);

        let expected = vec![
            r##"A long comment that spans over two"#]
#[doc=r#"lines."##,
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
            let suggestion_set = reflow(&ContentOrigin::TestEntityRust, chunk, &cfg)
                .expect("Reflow is working. qed");
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
            => r##"First line Second"#]
#[doc=r#"line third line"##, false);
    }

    #[test]
    fn reflow_doc_long() {
        reflow_chyrp!(40 break ["One line which is quite long and needs to be reflown in another line."]
            => r##"One line which is quite long"#]
#[doc=r#"and needs to be reflown in"#]
#[doc=r#"another line."##, false);
    }

    #[test]
    fn reflow_sole_markdown() {
        let _ = env_logger::Builder::new()
            .filter(None, log::LevelFilter::Debug)
            .is_test(true)
            .try_init();

        use std::convert::TryInto;

        const CONFIG: ReflowConfig = ReflowConfig {
            max_line_length: 60,
        };

        const CONTENT: &'static str =
            "# Possible __ways__ to run __rustc__ and request various parts of LTO.

A short line but long enough such that we reflow it. Yada lorem ipsum stuff needed.

- a list
- another point

Some <pre>Hmtl tags</pre>.

Some more text after the newline which **represents** a paragraph";

        const EXPECTED: &[(&'static str, Span)] = &[
            (
                r#"A short line but long enough such that we reflow it. Yada
lorem ipsum stuff needed."#,
                Span {
                    start: LineColumn { line: 3, column: 0 },
                    end: LineColumn {
                        line: 3,
                        column: 83,
                    },
                },
            ),
            (
                r#"Some more text after the newline which **represents** a
paragraph"#,
                Span {
                    start: LineColumn {
                        line: 10,
                        column: 0,
                    },
                    end: LineColumn {
                        line: 10,
                        column: 64,
                    },
                },
            ),
        ];

        let docs = Documentation::from((ContentOrigin::TestEntityCommonMark, CONTENT));
        assert_eq!(docs.entry_count(), 1);
        let chunks = docs
            .get(&ContentOrigin::TestEntityCommonMark)
            .expect("Contains test data. qed");
        assert_eq!(dbg!(chunks).len(), 1);
        let chunk = chunks.first().unwrap();

        let suggestion_set = reflow(&ContentOrigin::TestEntityCommonMark, &chunk, &CONFIG)
            .expect("Reflow is working. qed");
        assert_eq!(suggestion_set.len(), 2);

        for (sug, &(expected_content, expected_span)) in suggestion_set.iter().zip(EXPECTED.iter())
        {
            dbg!(&sug.span);
            dbg!(&sug.range);
            assert_eq!(sug.replacements.len(), 1);
            let replacement = sug
                .replacements
                .iter()
                .next()
                .expect("Reflow always provides a replacement string. qed");

            assert_eq!(sug.span, expected_span);

            assert_eq!(replacement.as_str(), expected_content);
        }
    }
}
