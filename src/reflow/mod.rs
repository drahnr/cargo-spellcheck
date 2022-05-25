//! Reflow documentation comments to a desired line width.
//!
//! Note that for commonmark this might not be possible with links. The reflow
//! is done based on the comments no matter the content.

use crate::checker::Checker;
use crate::documentation::CheckableChunk;
use crate::errors::{eyre, Result};
use crate::util::extract_delimiter;
#[cfg(debug_assertions)]
use crate::util::load_span_from;
use crate::util::{byte_range_to_char_range, byte_range_to_char_range_many, sub_char_range};

use crate::{CommentVariant, ContentOrigin, Detector, Range, Span, Suggestion};

use pulldown_cmark::{Event, Options, Parser, Tag};

pub use crate::config::ReflowConfig;

mod iter;
pub use iter::{Gluon, Tokeneer};

#[derive(Debug)]
pub struct Reflow {
    config: ReflowConfig,
}

impl Reflow {
    pub fn new(config: ReflowConfig) -> Result<Self> {
        Ok(Self { config })
    }
}

impl Checker for Reflow {
    type Config = ReflowConfig;

    fn detector() -> Detector {
        Detector::Reflow
    }

    fn check<'a, 's>(
        &self,
        origin: &ContentOrigin,
        chunks: &'a [CheckableChunk],
    ) -> Result<Vec<Suggestion<'s>>>
    where
        'a: 's,
    {
        let mut acc = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            match chunk.variant() {
                CommentVariant::SlashAsterisk
                | CommentVariant::SlashAsteriskAsterisk
                | CommentVariant::SlashAsteriskEM => continue,
                _ => {}
            }
            let suggestions = reflow(&origin, chunk, &self.config)?;
            acc.extend(suggestions);
        }
        Ok(acc)
    }
}

/// Reflows a parsed commonmark paragraph contained in `s`.
///
/// Returns the `Some(replacement)` string if a reflow has been performed and
/// `None` otherwise.
///
/// `range` denotes the range of the paragraph of interest in the top-level
/// `CheckableChunk`. `unbreakable_ranges` contains all ranges of
/// words/sequences which must not be split during the reflow. They are relative
/// to the top-level `CheckableChunk` similar to `range`. The indentation vector
/// contains the indentation for each line in `s`.
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
    let s_absolute = sub_char_range(s, range.clone());
    // now if the last character is a newline, we spare it, since it would be
    // annihilated by the `Tokeneer` without replacement.
    let mut sit = s.chars();
    let _first_char_is_newline = sit.next().map(|c| c == '\n').unwrap_or_default();
    // make sure we do not double count the \n in case of a single `\n` in `s`
    // by re-use of a single  iterator
    let last_char_is_newline = sit.last().map(|c| c == '\n').unwrap_or_default();

    let unbreakables = unbreakable_ranges
        .iter()
        .map(|r| (r.start.saturating_sub(range.start))..(r.end.saturating_sub(range.start)));

    let mut gluon = Gluon::new(s_absolute, max_line_width, &indentations);
    gluon.add_unbreakables(unbreakables);

    let mut reflow_applied = false;
    let mut lines = s_absolute.lines();
    let mut indents_iter = indentations.iter();
    let last_indent = indentations
        .last()
        .copied()
        .ok_or_else(|| eyre!("No line indentation present."))?;

    // First line has to be without indent and variant prefix.
    // If there is nothing to reflow, just pretend there was no reflow.
    let (_lineno, content, _range) = match gluon.next() {
        Some(c) => c,
        None => return Ok(None),
    };
    if lines.next() != Some(&content) {
        reflow_applied = true;
    }

    let mut acc = content.to_owned() + &variant.suffix_string();
    if !acc.is_empty() {
        acc.push_str(line_delimiter);
    }

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
        let mut content = content
            .strip_suffix(&variant.suffix_string())
            .map(|content| content.to_owned())
            .unwrap_or_else(|| content);
        if &CommentVariant::CommonMark == variant && last_char_is_newline && !content.is_empty() {
            content.push_str(line_delimiter)
        }

        // we might be constrained by the unbreakable in a way
        // that we cannot resolve the too long lines
        // and as such the reconstruncted content might be identical
        // in which case we don't want to bother with it any longer
        if content != s_absolute {
            log::debug!("Constraints of unbreakable sequences could not resolve too long lines");
            Some(content)
        } else {
            None
        }
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
            sub_char_range(s, 0..n).to_owned()
        } else {
            " ".repeat(self.offset.saturating_sub(n))
        }
    }
}

/// Collect reflown Paragraphs in a `Vec` of `Suggestions`.
///
/// Note: Leading spaces are skipped by the CommonMark parser, which implies for
/// `///` and `//!`, the paragraph for the first line starting right after `///
/// ` (note the space here).
///
///
/// Returns: end of processed range and Suggestion, if reflow happened.
fn store_suggestion<'s>(
    chunk: &'s CheckableChunk,
    origin: &ContentOrigin,
    bytes_paragraph: usize,
    bytes_end: usize,
    bytes_unbreakable_ranges: &[Range],
    max_line_width: usize,
) -> Result<(usize, Option<Suggestion<'s>>)> {
    let bytes_range = Range {
        start: bytes_paragraph,
        end: bytes_end,
    };
    let s = chunk.as_str();
    #[cfg(debug_assertions)]
    let sb = s.as_bytes();

    let unbreakable_ranges = byte_range_to_char_range_many(s, bytes_unbreakable_ranges);
    let unbreakable_ranges = unbreakable_ranges.as_slice();

    let range = byte_range_to_char_range(s, bytes_range.clone())
        .expect("Must have alignment to byte boundaries. qed");

    #[cfg(debug_assertions)]
    log::trace!(
        "reflow::store_suggestion(chunk([{:?}]): {:?}",
        &range,
        &s[bytes_range.clone()],
    );

    // with markdown, the initial paragraph for `/// `
    // might be shifted, so the start in those cases must be shifted back
    // to right after `///`, which is done by substracting one.
    let adjustment = match chunk.variant() {
        CommentVariant::DoubleSlashEM | CommentVariant::TripleSlash => 1usize,
        _ => 0usize,
    };

    debug_assert_eq!(&s[bytes_range], sub_char_range(s, range.clone()));

    let range2span = chunk.find_spans(range.clone());
    let mut spans_iter = range2span.iter().map(|(_range, span)| *span);

    let span = {
        let Span {
            start,
            end: fallback_end,
        } = if let Some(first) = spans_iter.next() {
            first
        } else {
            return Ok((bytes_paragraph, None));
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
        load_span_from(sb, span).unwrap()
    );

    // Get indentation for each span, if a span covers multiple
    // lines, use same indentation for all lines.
    let mut first = true;
    let indentations = range2span
        .iter()
        .flat_map(|(_range, span)| {
            debug_assert!(span.start.line <= span.end.line);

            // TODO use `sub_char_range(s, range.clone())`
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
        bytes_end,
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

/// Parses a `CheckableChunk` and performs the re-wrapping on contained
/// paragraphs.
fn reflow<'s>(
    origin: &ContentOrigin,
    chunk: &'s CheckableChunk,
    cfg: &ReflowConfig,
) -> Result<Vec<Suggestion<'s>>> {
    log::debug!("Reflowing {:?}", origin);
    let parser = Parser::new_ext(chunk.as_str(), Options::all());

    let mut paragraph = 0_usize;
    // nested unbreakables are tracked via a stack approach
    let mut unbreakable_stack: Vec<Range> = Vec::with_capacity(16); // no more than 16 items will be nested, commonly it's 2 or 3
                                                                    // the true unbreakables (without inner nested items)
                                                                    // to be used for reflowing
    let mut unbreakables = Vec::with_capacity(256);

    let mut acc = Vec::with_capacity(128);

    for (event, cover) in parser.into_offset_iter() {
        #[cfg(debug_assertions)]
        {
            log::trace!("CMark Token: {:?}", &event);
            log::trace!(
                "Current segment {:?}: {:?}",
                cover,
                &chunk.as_str()[cover.clone()]
            );
        }
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
                            unbreakables.as_slice(),
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
                        let _ = unbreakable_stack.pop();
                    }
                    Tag::Paragraph => {
                        // regular end of paragraph
                        let (p, suggestion) = store_suggestion(
                            chunk,
                            origin,
                            paragraph,
                            cover.end,
                            unbreakables.as_slice(),
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
            Event::Code(_s) => {
                // always make code unbreakable
                unbreakables.push(cover);
            }
            Event::Html(_s) => {
                unbreakables.push(cover);
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
                    unbreakables.as_slice(),
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
