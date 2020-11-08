//! Reflow documentation comments to a desired line width.
//!
//! Note that for commonmark this might not be possible with links.
//! The reflow is done based on the comments no matter the content.

use anyhow::{anyhow, Result};

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

#[derive(Debug, PartialEq, Eq)]
struct LineSepStat {
    first_appearance: usize,
    count: usize,
    newline: &'static str,
}

#[inline(always)]
fn extract_delimiter_inner<'a>(
    mut iter: impl Iterator<Item = regex::Match<'a>>,
    newline: &'static str,
) -> Option<LineSepStat> {
    if let Some(first) = iter.next() {
        let first = first.start();
        let n = iter.count() + 1;
        Some(LineSepStat {
            first_appearance: first,
            count: n,
            newline,
        })
    } else {
        None
    }
}

/// Extract line delimiter of a string
fn extract_delimiter<'s>(s: &'s str) -> Option<&'static str> {
    use regex::Regex;

    // TODO lots of room for optimizations here
    lazy_static::lazy_static! {
        static ref LF: Regex = Regex::new(r#"\n"#).expect("LF regex compiles. qed");
        static ref CR: Regex = Regex::new(r#"\r"#).expect("CR regex compiles. qed");
        static ref CRLF: Regex = Regex::new(r#"\r\n"#).expect("CRLF regex compiles. qed");
        static ref LFCR: Regex = Regex::new(r#"\n\r"#).expect("LFCR regex compiles. qed");
    };

    // first look for two letter line delimiters
    let lfcr = extract_delimiter_inner(LFCR.find_iter(s), "\n\r");
    let crlf = extract_delimiter_inner(CRLF.find_iter(s), "\r\n");

    // remove the 2 line line delimiters from the single line line delimiters, since they overlap
    let lf = extract_delimiter_inner(LF.find_iter(s), "\n").map(|mut stat| {
        stat.count = stat.count.saturating_sub(std::cmp::max(
            crlf.as_ref().map(|stat| stat.count).unwrap_or_default(),
            lfcr.as_ref().map(|stat| stat.count).unwrap_or_default(),
        ));
        stat
    });
    let cr = extract_delimiter_inner(CR.find_iter(s), "\r").map(|mut stat| {
        stat.count = stat.count.saturating_sub(std::cmp::max(
            crlf.as_ref().map(|stat| stat.count).unwrap_or_default(),
            lfcr.as_ref().map(|stat| stat.count).unwrap_or_default(),
        ));
        stat
    });

    // order is important, `max_by` prefers the latter ones over the earlier ones on equality
    vec![cr, lf, crlf, lfcr]
        .into_iter()
        .filter_map(|x| x)
        .max_by(|b, a| {
            if a.count == b.count {
                dbg!(a.first_appearance.cmp(&b.first_appearance))
            } else {
                b.count.cmp(&a.count)
            }
        })
        .map(|x| x.newline)
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
    indentations: &[Indentation<'s>],
    max_line_width: usize,
    variant: &CommentVariant,
) -> Result<Option<String>> {
    // Get type of newline from current chunk, either plain \n or \r\n
    let line_delimiter = extract_delimiter(s).unwrap_or_else(|| {
        // TODO if ther is no newline in `s`, we assume `\n`
        // TODO make this depend on the file
        log::warn!("Could not determine a line delimiter, falling back to \\n");
        "\n"
    });

    // extract the relevant part from the entire `chunk`, that will be our working set.
    let s_absolute = crate::util::sub_chars(s, range.clone());
    let unbreakables = unbreakable_ranges
        .iter()
        .map(|r| (r.start.saturating_sub(range.start))..(r.end.saturating_sub(range.start)));

    let mut gluon = Gluon::new(&s_absolute, max_line_width, &indentations);
    gluon.add_unbreakables(unbreakables);

    let mut reflow_applied = false;
    let mut lines = s_absolute.lines();
    let mut indents_iter = indentations.iter();
    let last_indent = indentations
        .last()
        .copied()
        .ok_or_else(|| anyhow!("No line indentation present."))?;

    // First line has to be without indent and variant prefix.
    // If there is nothing to reflow, just pretend there was no reflow.
    let (_lineno, content, _range) = match gluon.next() {
        Some(c) => c,
        None => return Ok(None),
    };
    if lines.next() != Some(&content) {
        reflow_applied = true;
    }
    let acc = content + &variant.suffix_string() + line_delimiter;

    // construct replacement string from prefix and Gluon iterations
    let content = gluon.fold(acc, |mut acc, (_lineno, content, _range)| {
        if lines.next() == Some(&content) {
            reflow_applied = true;
        }

        // avoid stray spaces after newlines due to a truely required indentation
        // of 3 for `///` but practically, it's `/// ` (added space), which should be accounted for,
        // since that is used for accounting for the skip covered by `///`,
        // which is being removed by the transformation `s` to `s_absolute`
        // that removes the leading space.
        let (indentation_skip_n, extra_space) = match variant {
            CommentVariant::TripleSlash | CommentVariant::DoubleSlashEM => {
                let n = variant.prefix_len();
                (n + 1, " ")
            }
            _ => (variant.prefix_len(), ""),
        };
        let pre = if let Some(indentation) = indents_iter.next() {
            indentation
        } else {
            &last_indent
        }
        .to_string_but_skip_n(indentation_skip_n);

        log::trace!(target: "glue", "glue[shift={}]: acc = {:?} + {:?} + {:?} + {:?} + {:?} + {:?}",
                indentation_skip_n,
                &pre,
                &variant.prefix_string(),
                extra_space,
                &content,
                &variant.suffix_string(),
                line_delimiter
        );
        acc.push_str(&pre);
        acc.push_str(&variant.prefix_string());
        acc.push_str(extra_space);
        acc.push_str(&content);
        acc.push_str(&variant.suffix_string());
        acc.push_str(line_delimiter);
        acc
    });

    // remove last new line
    let content = if let Some(c) = content.strip_suffix(line_delimiter) {
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

#[derive(Default, Debug, Hash, Eq, PartialEq, Copy, Clone)]
pub(crate) struct Indentation<'s> {
    offset: usize,
    s: Option<&'s str>,
}

impl<'s> ToString for Indentation<'s> {
    fn to_string(&self) -> String {
        if let Some(s) = self.s {
            s.to_owned()
        } else {
            " ".repeat(self.offset)
        }
    }
}

impl<'s> Indentation<'s> {
    pub(crate) fn new(offset: usize) -> Self {
        log::trace!("New offset with indentation of {} ", offset);
        Self { offset, s: None }
    }

    #[allow(unused)]
    pub(crate) fn with_str(offset: usize, s: &'s str) -> Self {
        log::trace!("New offset with indentation of {} and {:?}", offset, s);
        Self { offset, s: Some(s) }
    }

    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    #[allow(unused)]
    /// Convert to a string but skip `n` chars
    pub(crate) fn to_string_but_skip_n(&self, n: usize) -> String {
        if let Some(s) = self.s {
            dbg!(crate::util::sub_char_range(s, 0..dbg!(n)).to_owned())
        } else {
            dbg!(" ".repeat(dbg!(self.offset).saturating_sub(dbg!(n))))
        }
    }
}

/// Collect reflowed Paragraphs in a `Vec` of `Suggestions`.
///
/// Note: Leading spaces are skipped by the markdown parser,
/// which implies for `///` and `//!`, the paragraph for the first
/// line starting right after `/// ` (note the space here).
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
    let s = chunk.as_str();
    #[cfg(debug_assertions)]
    let sb = s.as_bytes();

    #[cfg(debug_assertions)]
    log::trace!(
        "reflow::store_suggestion(chunk([{:?}]): {:?}",
        &range,
        crate::util::sub_char_range(s, range.clone()),
    );

    /// with markdown, the initial paragraph for `/// `
    /// might be shifted, so the start in those cases must be shifted back
    /// to right after `///`, which is done by substracting one.
    let adjustment = match chunk.variant() {
        CommentVariant::DoubleSlashEM | CommentVariant::TripleSlash => 1usize,
        _ => 0usize,
    };

    let range2span = chunk.find_spans(range.clone());
    let mut spans_iter = range2span.iter().map(|(_range, span)| *span);

    let span = {
        let Span { start, end: fallback_end} = if let Some(mut first) = spans_iter.next() {
            first.start.column = first.start.column.saturating_sub(adjustment);
            first
        } else {
            return Ok((paragraph, None));
        };
        let end = if let Some(last) = spans_iter.last() {
            last.end
        } else {
            fallback_end
        };

        Span { start, end }
    };

    #[cfg(debug_assertions)]
    log::trace!(
        "reflow::store_suggestion[source({:?})]: {:?}",
        span.clone(),
        crate::util::load_span_from(sb, span).unwrap()
    );

    // Get indentation for each span, if a span covers multiple
    // lines, use same indentation for all lines
    let mut first = true;
    let indentations = range2span
        .iter()
        .flat_map(|(_range, span)| {
            #[cfg(debug_assertions)]
            {
                dbg!(_range);
            }
            debug_assert!(span.start.line <= span.end.line);

            // TODO use crate::util::sub_char_range(s, range.clone())
            // TODO and `Indent::with_str(..)`

            let col = span
                .start
                .column
                .saturating_sub(dbg!(adjustment) * dbg!((first == true) as usize))
                + adjustment;
            let indentation = Indentation::new(col);
            first = false;
            vec![dbg!(indentation); dbg!(span.end.line.saturating_sub(span.start.line) + 1)]
        })
        .collect::<Vec<Indentation>>();

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

            let _ = env_logger::Builder::new()
                .filter(None, log::LevelFilter::Debug)
                .is_test(true)
                .try_init();

            const CONTENT: &'static str = fluff_up!($( $line ),+);
            let docs = Documentation::from((ContentOrigin::TestEntityRust, CONTENT));
            assert_eq!(docs.entry_count(), 1);
            let chunks = docs.get(&ContentOrigin::TestEntityRust).expect("Must contain dummy path");
            assert_eq!(dbg!(chunks).len(), 1);
            let chunk = &chunks[0];

            let range = 0..chunk.as_str().len();
            let indentation: Vec<_> = [3; 6].iter().map(|&n| {
                Indentation::<'static>::new(n)
            }).collect::<Vec<_>>();
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

    /// Run reflow on a set of lines that are `fluff_up`ed
    /// and match the resulting `Patch`s replacment with
    /// an `expected`.
    macro_rules! reflow {
        ([ $( $line:literal ),+ $(,)?] => $expected:literal, $no_reflow:expr) => {
            reflow!(80usize break [ $( $line ),+ ] => $expected, $no_reflow:expr);
        };
        ($n:literal break [ $( $line:literal ),+ $(,)?] => $expected:literal, $no_reflow:expr) => {
            let _ = env_logger::Builder::new()
                .filter(None, log::LevelFilter::Debug)
                .is_test(true)
                .try_init();

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
                    log::info!("Replacement {:?}", replacement);
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
            let _ = env_logger::Builder::new()
                .filter(None, log::LevelFilter::Debug)
                .is_test(true)
                .try_init();

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
        reflow!(45 break ["This module contains documentation thats \
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
        let _ = env_logger::Builder::new()
            .filter_level(log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        const CONTENT: &'static str = r#"
    /// 🔴 🍁
    /// 🤔
    struct Fluffy {};"#;

        const EXPECTED: &'static str = r#"🔴
    /// 🍁
    /// 🤔"#;

        const CONFIG: ReflowConfig = ReflowConfig {
            max_line_length: 10,
        };

        let docs = Documentation::from((ContentOrigin::TestEntityRust, CONTENT));
        assert_eq!(docs.entry_count(), 1);
        let chunks = docs
            .get(&ContentOrigin::TestEntityRust)
            .expect("Contains test data. qed");
        assert_eq!(dbg!(chunks).len(), 1);
        let chunk = &chunks[0];

        let suggestion_set =
            reflow(&ContentOrigin::TestEntityRust, chunk, &CONFIG).expect("Reflow is wokring. qed");

        let suggestion = suggestion_set
            .iter()
            .next()
            .expect("Contains one suggestion. qed");

        dbg!(crate::util::load_span_from(&mut CONTENT.as_bytes(), suggestion.span).unwrap());

        let replacement = suggestion
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
    fn reflow_fold_two_to_one() {
        reflow!(20 break ["A 🚤>", "<To 🌴/🍉&🍈"]
                => "A 🚤> <To 🌴/🍉&🍈",
                false);
    }

    #[test]
    fn reflow_split_one_into_three() {
        reflow!(9 break ["A 🌴xX 🍉yY 🍈zZ"]
                => "A 🌴xX\n/// 🍉yY\n/// 🍈zZ",
                false);
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
        const CONFIG: ReflowConfig = ReflowConfig {
            max_line_length: 60,
        };

        const CONTENT: &'static str =
            "# Possible __ways__ to run __rustc__ and request various parts of LTO.

A short line but long enough such that we reflow it. Yada lorem ipsum stuff needed.

- a list
- another point

Some <pre>Hmtl tags</pre>.

Some more text after the newline which **represents** a paragraph
in two lines. In my opinion paraghraphs are always multiline. Fullstop.";

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
paragraph in two lines. In my opinion paraghraphs are always
multiline. Fullstop."#,
                Span {
                    start: LineColumn {
                        line: 10,
                        column: 0,
                    },
                    end: LineColumn {
                        line: 11,
                        column: 70,
                    },
                },
            ),
        ];

        let _ = env_logger::Builder::new()
            .filter(None, log::LevelFilter::Debug)
            .is_test(true)
            .try_init();

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

    #[test]
    fn reflow_line_delimiters() {
        const TEST_DATA: &[(&'static str, &'static str)] = &[
            ("Two lines\nhere", "\n"),
            ("Two lines\r\nhere", "\r\n"),
            ("\r\n\r\n", "\r\n"),
            ("\n\r\n\r\n", "\n\r"),
            ("\n\n\n\r\n", "\n"),
            ("\n\r\n\n\r\n", "\n\r"),
            ("Two lines\n\rhere", "\n\r"),
            ("Two lines\nhere\r\nand more\r\nsfd", "\r\n"),
            ("Two lines\r\nhere\nand more\n", "\n"),
            ("Two lines\nhere\r\nand more\n\r", "\n"),
            ("Two lines\nhere\r\nand more\n", "\n"),
        ];
        for (input, expected) in TEST_DATA {
            let expected = *expected;
            println!("{:?} should detect {:?}", input, expected);
            assert_eq!(extract_delimiter(input), Some(expected));
        }
    }
}
