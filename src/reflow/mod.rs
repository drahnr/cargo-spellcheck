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

/// Extract line delimiter of a string.
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
                a.first_appearance.cmp(&b.first_appearance)
            } else {
                b.count.cmp(&a.count)
            }
        })
        .map(|x| x.newline)
}

/// Reflows a parsed commonmark paragraph contained in `s`.
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

    /// Convert to a string but skip `n` chars
    pub(crate) fn to_string_but_skip_n(&self, n: usize) -> String {
        if let Some(s) = self.s {
            crate::util::sub_char_range(s, 0..n).to_owned()
        } else {
            " ".repeat(self.offset.saturating_sub(n))
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

    // with markdown, the initial paragraph for `/// `
    // might be shifted, so the start in those cases must be shifted back
    // to right after `///`, which is done by substracting one.
    let adjustment = match chunk.variant() {
        CommentVariant::DoubleSlashEM | CommentVariant::TripleSlash => 1usize,
        _ => 0usize,
    };

    let range2span = chunk.find_spans(range.clone());
    let mut spans_iter = range2span.iter().map(|(_range, span)| *span);

    let span = {
        let Span {
            start,
            end: fallback_end,
        } = if let Some(first) = spans_iter.next() {
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
            debug_assert!(span.start.line <= span.end.line);

            // TODO use crate::util::sub_char_range(s, range.clone())
            // TODO and `Indent::with_str(..)`

            // Adjust the column by adding the adjustment to every line
            // but the first. Necessary, since cmark swallows leading whitespace
            // but the following leading whitespaces of literals in the same
            // chunk are still present, yet they are not part of the prefix
            // as defined by the `CommentVariant` for `///` and `//!`.
            let col = span
                .start
                .column
                .saturating_sub(adjustment * (first as usize))
                + adjustment;
            let indentation = Indentation::new(col);
            first = false;
            vec![indentation; span.end.line.saturating_sub(span.start.line) + 1]
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
mod tests;
