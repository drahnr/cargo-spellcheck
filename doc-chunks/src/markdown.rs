//! Erase cmark syntax
//!
//! Resulting overlay is plain and can be fed into a grammar or spell checker.

use super::*;

use indexmap::IndexMap;

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};

use crate::util::sub_chars;
use crate::Span;
use crate::{CheckableChunk, Range};

/// Describes whether there is a matching segment in the source, of if it is a
/// placeholder for i.e. a code block or inline code. These placeholders are
/// required for grammar checks.
#[derive(Debug, Clone)]
pub enum SourceRange {
    Direct(Range),
    Alias(Range, String),
}

impl SourceRange {
    /// Apply an offset to `start` and `end` members, equaling a shift of the
    /// range.
    #[allow(dead_code)]
    pub(crate) fn apply_offset(&mut self, offset: usize) {
        match self {
            Self::Direct(range) => apply_offset(range, offset),
            Self::Alias(range, _) => apply_offset(range, offset),
        }
    }

    /// Extract a clone of the inner `Range<usize>`.
    ///
    /// Use `deref()` or `*` for a reference.
    pub fn range(&self) -> Range {
        match self {
            Self::Direct(range) => range.clone(),
            Self::Alias(range, _) => range.clone(),
        }
    }
}

impl std::ops::Deref for SourceRange {
    type Target = Range;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Direct(range) => range,
            Self::Alias(range, _) => range,
        }
    }
}

pub(crate) fn is_html_tag_on_no_scope_list(text: &str) -> bool {
    use regex::RegexSet;
    lazy_static::lazy_static! {
        static ref HTML_TAG_EMPTY_OR_SPECIAL_CASE: RegexSet = RegexSet::new(&[
            r####"^<\s*[A-Za-z0-9]+(?:\s+.*)*\s*/>$"####, // any self closing empty
            r####"^<\s*br\s*>$"####,
            r####"^</?\s*(?:i|b|span|font|color|style)\s*/?>$"####,
            r####"^<\s*pre\s*>.*</\s*pre\s*>\s?$"####,
        ]).unwrap();
    };
    HTML_TAG_EMPTY_OR_SPECIAL_CASE.is_match(text)
}

#[test]
fn scoped() {
    assert_eq!(false, is_html_tag_on_no_scope_list("<code>"));
    assert_eq!(false, is_html_tag_on_no_scope_list("</code>"));
    assert_eq!(true, is_html_tag_on_no_scope_list("<code />"));
    assert_eq!(true, is_html_tag_on_no_scope_list("<pre>🌡</pre>\n"));
}

/// A plain representation of cmark riddled chunk.
#[derive(Clone)]
pub struct PlainOverlay<'a> {
    /// A reference to the underlying [`CheckableChunk`][super::chunk].
    raw: &'a CheckableChunk,
    /// The rendered string with all common mark annotations removed.
    plain: String,
    // require a sorted map, so we have the chance of binary search
    // key: plain string range
    // value: the corresponding areas in the full cmark
    mapping: IndexMap<Range, SourceRange>,
}

impl<'a> PlainOverlay<'a> {
    /// Track the origin of the annotation free content string fragments in the
    /// common mark formatted text, to the fragments in the plain string.
    fn track(
        s: &str,
        cmark_range: SourceRange,
        plain_acc: &mut String,
        mapping: &mut IndexMap<Range, SourceRange>,
    ) {
        // map the range within the plain data,
        // which is fed to the checker,
        // back to the repr with markdown modifiers

        // avoid repeated calculation of this
        let cursor = plain_acc.chars().count();
        let plain_range = match &cmark_range {
            SourceRange::Alias(_range, alias) => {
                if alias.is_empty() {
                    log::debug!("Alias for {s:?} was empty. Ignoring.");
                    return;
                }
                // limit the lias names to 16 chars, all ascii
                // and as such byte length equals char length
                let alias16 = &alias[..std::cmp::min(alias.len(), 16)];
                plain_acc.push_str(alias16);
                Range {
                    start: cursor,
                    end: cursor + alias16.len(),
                }
            }
            SourceRange::Direct(_range) => {
                plain_acc.push_str(&s);
                Range {
                    start: cursor,
                    end: cursor + s.chars().count(),
                }
            }
        };
        let _ = mapping.insert(plain_range, cmark_range);
    }

    /// Append n newlines to the current state string `plain`.
    fn newlines(plain: &mut String, n: usize) {
        for _ in 0..n {
            plain.push('\n');
        }
    }

    /// Ranges are mapped `cmark reduced/plain -> raw`.
    pub fn extract_plain_with_mapping(
        cmark: &str,
        ignores: &Ignores,
    ) -> (String, IndexMap<Range, SourceRange>) {
        let mut plain = String::with_capacity(cmark.len());
        let mut mapping = indexmap::IndexMap::with_capacity(128);

        let broken_link_handler = &mut |_broken: pulldown_cmark::BrokenLink| -> Option<(
            pulldown_cmark::CowStr,
            pulldown_cmark::CowStr,
        )> {
            Some((
                pulldown_cmark::CowStr::Borrowed(""),
                pulldown_cmark::CowStr::Borrowed(""),
            ))
        };
        let parser = Parser::new_with_broken_link_callback(
            cmark,
            Options::all() ^ Options::ENABLE_SMART_PUNCTUATION,
            Some(broken_link_handler),
        );

        let rust_fence =
            pulldown_cmark::CodeBlockKind::Fenced(pulldown_cmark::CowStr::Borrowed("rust"));

        let mut html_block = 0_usize;
        let mut code_block = 0_usize;
        let mut html_code_block = 0_usize;
        let mut inception = false;
        let mut skip_link_text = false;
        let mut skip_table_text = false;

        for (event, byte_range) in parser.into_offset_iter() {
            if byte_range.start > byte_range.end {
                log::warn!(
                    "Dropping event {event:?} due to negative byte range {byte_range:?}, see {}",
                    "https://github.com/raphlinus/pulldown-cmark/issues/478"
                );
                continue;
            }

            log::trace!("Parsing event (bytes: {byte_range:?}): {event:?}");

            let mut cursor = cmark.char_indices().enumerate().peekable();
            let mut char_cursor = 0usize;

            // let the cursor catch up to the current byte position
            while let Some((char_idx, (byte_offset, _c))) = cursor.next() {
                char_cursor = char_idx;
                if byte_offset >= byte_range.start {
                    break;
                }
            }
            // convert to a character range given the char_cursor
            // TODO defer the length calculation into the tags, where the string is already extracted.
            let char_range = {
                let bytes_start = std::cmp::min(byte_range.start, cmark.len());
                let bytes_end = std::cmp::min(byte_range.end, cmark.len());
                assert!(bytes_start <= bytes_end);
                let char_count = cmark[bytes_start..bytes_end].chars().count();
                char_cursor..(char_cursor + char_count)
            };

            match event {
                Event::InlineHtml(html) => {
                    if html.starts_with("<code") {
                        html_code_block += 1;
                    } else if html.ends_with("code>") {
                        html_code_block = html_code_block.saturating_sub(1);
                    }
                }
                Event::Start(tag) => match tag {
                    Tag::Table(_alignments) => {
                        skip_table_text = true;
                    }
                    Tag::TableCell | Tag::TableHead | Tag::TableRow => {}
                    Tag::CodeBlock(fenced) => {
                        code_block += 1;
                        inception = fenced == rust_fence;
                    }
                    Tag::Link {
                        link_type,
                        dest_url: _,
                        title: _,
                        id: _,
                    } => {
                        skip_link_text = match link_type {
                            LinkType::ReferenceUnknown
                            | LinkType::Reference
                            | LinkType::Inline
                            | LinkType::Collapsed
                            | LinkType::CollapsedUnknown
                            | LinkType::Shortcut
                            | LinkType::ShortcutUnknown => false,
                            LinkType::Autolink | LinkType::Email => true,
                        };
                    }
                    Tag::List(_) => {
                        // make sure nested lists are not clumped together
                        Self::newlines(&mut plain, 1);
                    }
                    Tag::Image {
                        link_type: _,
                        dest_url: _,
                        title,
                        id: _,
                    } => {
                        Self::track(
                            &title,
                            SourceRange::Direct(char_range),
                            &mut plain,
                            &mut mapping,
                        );
                    }
                    _ => {}
                },
                Event::End(tag) => {
                    match tag {
                        TagEnd::Table { .. } => {
                            skip_table_text = false;
                            Self::newlines(&mut plain, 1);
                        }
                        TagEnd::Link => {
                            // the actual rendered content is in a text section
                        }
                        TagEnd::Image => {}
                        TagEnd::Heading(_level) => {
                            Self::newlines(&mut plain, 2);
                        }
                        TagEnd::CodeBlock => {
                            code_block = code_block.saturating_sub(1);

                            // if fenced == rust_fence {
                            // TODO validate as if it was another document entity
                            // }
                        }
                        TagEnd::Paragraph => Self::newlines(&mut plain, 2),

                        TagEnd::Item => {
                            // assure individual list items are not clumped together
                            Self::newlines(&mut plain, 1);
                        }
                        _ => {}
                    }
                }
                Event::Text(s) => {
                    if html_block > 0 {
                    } else if html_code_block > 0 {
                    } else if code_block > 0 {
                        if inception {
                            // let offset = char_range.start;
                            // TODO validate as additional, virtual document
                            // TODO https://github.com/drahnr/cargo-spellcheck/issues/43
                            // FIXME must also run the whole syn/ra_syntax pipeline not just another mapping
                            // let (inner, inner_mapping) = Self::extract_plain_with_mapping(s.as_str());
                            // mapping.extend(inner_mapping.into_iter().map(|(mut k,mut v)|
                            //     {
                            //         apply_offset(&mut k, offset);
                            //         v.apply_offset(offset);
                            //         (k,v)
                            //     }));
                            // plain.push_str(dbg!(inner.as_str()));
                        }
                    } else if skip_link_text {
                        skip_link_text = false
                    } else if !skip_table_text {
                        Self::track(
                            &s,
                            SourceRange::Direct(char_range),
                            &mut plain,
                            &mut mapping,
                        );
                    }
                }
                Event::Code(s) => {
                    // inline code such as `YakShave` shall be ignored, but we must keep a placeholder for grammar
                    // rules to avoid misleading suggestions.
                    let shortened_range = Range {
                        start: char_range.start.saturating_add(1),
                        end: char_range.end.saturating_sub(1),
                    };
                    let alias = cmark[byte_range]
                        .chars()
                        .skip(1)
                        .take(shortened_range.len())
                        .filter(|x| x.is_ascii_alphanumeric())
                        .collect::<String>();

                    if !shortened_range.is_empty() && !alias.is_empty() {
                        Self::track(
                            &s,
                            SourceRange::Alias(shortened_range, alias),
                            &mut plain,
                            &mut mapping,
                        );
                    }
                }
                Event::Html(tag) => {
                    if is_html_tag_on_no_scope_list(&tag) {
                    } else if tag.ends_with("/>") {
                        html_block = html_block.saturating_sub(1);
                    } else {
                        html_block += 1;
                    }
                }
                Event::FootnoteReference(s) => {
                    if !ignores.footnote_references && !s.is_empty() {
                        let char_range = Range {
                            start: char_range.start + 2,
                            end: char_range.end - 1,
                        };
                        Self::track(
                            &s,
                            SourceRange::Direct(char_range),
                            &mut plain,
                            &mut mapping,
                        );
                    }
                }
                Event::SoftBreak => {
                    Self::newlines(&mut plain, 1);
                }
                Event::HardBreak => {
                    Self::newlines(&mut plain, 2);
                }
                Event::Rule => {
                    Self::newlines(&mut plain, 1);
                }
                Event::TaskListMarker(_checked) => {}
            }
        }

        // the parser yields single lines as a paragraph, for which we add trailing newlines
        // which are pointless and clutter the test strings, so track and remove them
        let trailing_newlines = plain.chars().rev().take_while(|x| *x == '\n').count();
        if trailing_newlines <= plain.len() {
            plain.truncate(plain.len() - trailing_newlines)
        }
        if let Some((mut plain_range, raw_range)) = mapping.pop() {
            if plain_range.end > plain.len() {
                plain_range.end = plain.len();
            }
            if plain_range.start > plain_range.end {
                let content = String::from_iter(
                    cmark
                        .char_indices()
                        .filter(|(idx, _c)| raw_range.contains(idx))
                        .map(|(_idx, c)| c),
                );
                panic!(
                    "failed: {} <= {}, raw range: {:?}\ncontent: >>{}<<",
                    plain_range.start, plain_range.end, raw_range, content
                );
            }
            mapping.insert(plain_range, raw_range);
        }
        (plain, mapping)
    }

    /// Create a common mark overlay based on the provided `CheckableChunk`
    /// reference.
    // TODO consider returning a `Vec<PlainOverlay<'a>>` to account for list items
    // or other non-linear information which might not pass a grammar check as a whole
    pub fn erase_cmark(chunk: &'a CheckableChunk, ignores: &Ignores) -> Self {
        let (plain, mapping) = Self::extract_plain_with_mapping(chunk.as_str(), ignores);
        Self {
            raw: chunk,
            plain,
            mapping,
        }
    }

    /// Since most checkers will operate on the plain data, an indirection to
    /// map cmark reduced / plain back to raw ranges, which are then mapped back
    /// to `Span`s. The returned key `Ranges` are in the condensed domain.
    pub fn find_spans(&self, condensed_range: Range) -> IndexMap<Range, Span> {
        let mut active = false;
        let Range { start, end } = condensed_range;
        let n = self.mapping.len();
        self.mapping
            .iter()
            .skip_while(|(sub, _raw)| sub.end <= start)
            .take_while(|(sub, _raw)| sub.start < end)
            .inspect(|x| {
                log::trace!(">>> item {:?} ∈ {:?}", condensed_range, x.0);
            })
            .filter(|(sub, _)| {
                // could possibly happen on empty documentation lines with `///`
                !sub.is_empty()
            })
            .filter(|(_, raw)| {
                // aliases are not required for span search
                if let SourceRange::Direct(_) = raw {
                    true
                } else {
                    false
                }
            })
            .fold(
                IndexMap::<Range, Span>::with_capacity(n),
                |mut acc, (sub, raw)| {
                    fn recombine(range: Range, offset: usize, len: usize) -> Range {
                        Range {
                            start: range.start + offset,
                            end: range.start + offset + len,
                        }
                    }

                    let _ = if sub.contains(&start) {
                        // calculate the offset between our `condensed_range.start` and
                        // the `sub` which is one entry in the mappings
                        let offset = start - sub.start;
                        let overlay_range = if sub.contains(&(end - 1)) {
                            // complete start to end
                            active = false;
                            start..end
                        } else {
                            // only start, continue taking until end
                            active = true;
                            start..sub.end
                        };
                        let raw = recombine(raw.range(), offset, overlay_range.len());
                        Some((overlay_range, raw))
                    // TODO must be implemented properly
                    // } else if active {
                    //     let offset = sub.end - end;
                    //     if sub.contains(&(end - 1)) {
                    //         active = false;
                    //         Some((sub.start..end, offset))
                    //     } else {
                    //         Some((sub.clone(), offset))
                    //     }
                    } else {
                        None
                    }
                    .and_then(|(sub, raw)| {
                        log::trace!("convert:  cmark-erased={sub:?} -> raw={raw:?}");

                        if raw.is_empty() {
                            log::warn!("linear range to spans: {raw:?} empty!");
                        } else {
                            let resolved = self.raw.find_spans(raw.clone());
                            log::trace!("cmark-erased range to spans: {raw:?} -> {resolved:?}");
                            acc.extend(resolved.into_iter());
                        }
                        Some(())
                    });
                    acc
                },
            )
    }

    /// Obtains a reference to the plain, cmark erased representation.
    pub fn as_str(&self) -> &str {
        self.plain.as_str()
    }
}

use std::fmt;

impl<'a> fmt::Display for PlainOverlay<'a> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.plain.as_str())
    }
}

impl<'a> fmt::Debug for PlainOverlay<'a> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use console::Style;

        let styles = vec![
            Style::new().italic().bold().dim().red(),
            Style::new().italic().bold().dim().green(),
            Style::new().italic().bold().dim().yellow(),
            Style::new().italic().bold().dim().magenta(),
            Style::new().italic().bold().dim().cyan(),
        ];

        let uncovered = Style::new().bold().on_black().dim().white();

        let color_cycle = styles.iter().cycle();

        let commonmark = self.raw.as_str().to_owned();

        let mut coloured_plain = String::with_capacity(1024);
        let mut coloured_md = String::with_capacity(1024);

        let mut previous_md_end = 0usize;
        for (plain_range, md_range, style) in
            itertools::cons_tuples(self.mapping.iter().zip(color_cycle))
        {
            // TODO do this properly, `saturating sub` just prevents crashing
            let delta = md_range.start.saturating_sub(previous_md_end);
            // take care of the markers and things that are not rendered
            if delta > 0 {
                let s = sub_chars(commonmark.as_str(), previous_md_end..md_range.start);
                coloured_md.push_str(uncovered.apply_to(s.as_str()).to_string().as_str());
            }
            previous_md_end = md_range.end;

            let s = sub_chars(commonmark.as_str(), md_range.range());
            coloured_md.push_str(style.apply_to(s.as_str()).to_string().as_str());

            let s = sub_chars(self.plain.as_str(), plain_range.clone());
            coloured_plain.push_str(style.apply_to(s.as_str()).to_string().as_str());
        }
        // write!(formatter, "{coloured_md}")?;

        writeln!(formatter, "Commonmark:\n{coloured_md}")?;
        writeln!(formatter, "Plain:\n{coloured_plain}")?;
        Ok(())
    }
}

/// Explicitly ignored markdown entities.  The `Default` implementation means we
/// do not ignore anything, which is the backwards compatible configuration.
#[derive(Clone, Default)]
pub struct Ignores {
    /// Ignore [footnote references](Event::FootnoteReference).
    pub footnote_references: bool,
}
