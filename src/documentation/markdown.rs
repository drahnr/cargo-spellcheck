//! Erase cmark syntax
//!
//! Resulting overlay is plain and can be fed into a grammar or spell checker.

use super::*;

use indexmap::IndexMap;
use log::trace;
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};

use crate::documentation::{CheckableChunk, Range};
use crate::util::sub_chars;
use crate::Span;

/// A plain representation of cmark riddled set of a chunk.
#[derive(Clone)]
pub struct PlainOverlay<'a> {
    /// A reference to the underlying [`CheckableChunk`][super::chunk].
    raw: &'a CheckableChunk,
    /// The rendered string with all common mark annotations removed.
    plain: String,
    // require a sorted map, so we have the chance of binary search
    // key: plain string range
    // value: the corresponding areas in the full cmark
    mapping: IndexMap<Range, Range>,
}

impl<'a> PlainOverlay<'a> {
    /// Track the origin of the annotation free content string fragments in the common mark
    /// formatted text, to the fragments in the plain string.
    fn track(
        s: &str,
        cmark_range: Range,
        plain_acc: &mut String,
        mapping: &mut IndexMap<Range, Range>,
    ) {
        // map the range within the plain data,
        // which is fed to the checker,
        // back to the repr with markdown modifiers

        // TODO avoid doing this repeatedly, use a cursor
        let x = plain_acc.chars().count();
        let d = s.chars().count();
        let plain_range = Range {
            start: x,
            end: x + d,
        };
        let _ = mapping.insert(plain_range, cmark_range);
        plain_acc.push_str(&s);
    }

    /// Append n newlines to the current state string `plain`.
    fn newlines(plain: &mut String, n: usize) {
        for _ in 0..n {
            plain.push('\n');
        }
    }

    /// Ranges are mapped `cmark reduced/plain -> raw`.
    fn extract_plain_with_mapping(cmark: &str) -> (String, IndexMap<Range, Range>) {
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
            Options::ENABLE_TABLES
                | Options::ENABLE_FOOTNOTES
                | Options::ENABLE_STRIKETHROUGH
                | Options::ENABLE_TASKLISTS,
            Some(broken_link_handler),
        );

        let rust_fence =
            pulldown_cmark::CodeBlockKind::Fenced(pulldown_cmark::CowStr::Borrowed("rust"));

        let mut code_block = false;
        let mut inception = false;
        let mut skip_link_text = false;
        let mut skip_table_text = false;

        for (event, byte_range) in parser.into_offset_iter() {
            if byte_range.start > byte_range.end {
                warn!(
                    "Dropping event {:?} due to negative byte range {:?}, see {}",
                    event, byte_range, "https://github.com/raphlinus/pulldown-cmark/issues/478"
                );
                continue;
            }

            trace!("Parsing event (bytes: {:?}): {:?}", &byte_range, &event);

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
                Event::Start(tag) => match tag {
                    Tag::Table(_alignments) => {
                        skip_table_text = true;
                    }
                    Tag::TableCell | Tag::TableHead | Tag::TableRow => {}
                    Tag::CodeBlock(fenced) => {
                        code_block = true;
                        inception = fenced == rust_fence;
                    }
                    Tag::Link(link_type, _url, _title) => {
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
                    _ => {}
                },
                Event::End(tag) => {
                    match tag {
                        Tag::Table(_) => {
                            skip_table_text = false;
                            Self::newlines(&mut plain, 1);
                        }
                        Tag::Link(_link_type, _url, _title) => {
                            // the actual rendered content is in a text section
                        }
                        Tag::Image(_link_type, _url, title) => {
                            Self::track(&title, char_range, &mut plain, &mut mapping);
                        }
                        Tag::Heading(_n) => {
                            Self::newlines(&mut plain, 2);
                        }
                        Tag::CodeBlock(fenced) => {
                            code_block = false;

                            if fenced == rust_fence {
                                // TODO validate as if it was another document entity
                            }
                        }
                        Tag::Paragraph => Self::newlines(&mut plain, 2),

                        Tag::Item => {
                            // assure individual list items are not clumped together
                            Self::newlines(&mut plain, 1);
                        }
                        _ => {}
                    }
                }
                Event::Text(s) => {
                    if code_block {
                        if inception {
                            // TODO validate as additional, virtual document
                        }
                    } else if skip_link_text {
                        skip_link_text = false
                    } else if !skip_table_text {
                        Self::track(&s, char_range, &mut plain, &mut mapping);
                    }
                }
                Event::Code(_s) => {
                    // inline code such as `YakShave` shall be ignored
                }
                Event::Html(_s) => {}
                Event::FootnoteReference(s) => {
                    if !s.is_empty() {
                        let char_range = Range {
                            start: char_range.start + 2,
                            end: char_range.end - 1,
                        };
                        Self::track(&s, char_range, &mut plain, &mut mapping);
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
            assert!(plain_range.start <= plain_range.end);
            mapping.insert(plain_range, raw_range);
        }
        (plain, mapping)
    }

    /// Create a common mark overlay based on the provided `CheckableChunk` reference.
    // TODO consider returning a Vec<PlainOverlay<'a>> to account for list items
    // or other non-linear information which might not pass a grammar check as a whole
    pub fn erase_cmark(chunk: &'a CheckableChunk) -> Self {
        let (plain, mapping) = Self::extract_plain_with_mapping(chunk.as_str());
        Self {
            raw: chunk,
            plain,
            mapping,
        }
    }

    /// Since most checkers will operate on the plain data, an indirection to map cmark reduced / plain
    /// back to raw ranges, which are then mapped back to `Span`s.
    /// The returned key `Ranges` are in the condensed domain.
    pub fn find_spans(&self, condensed_range: Range) -> IndexMap<Range, Span> {
        let mut active = false;
        let Range { start, end } = condensed_range;
        self.mapping
            .iter()
            .skip_while(|(sub, _raw)| sub.end <= start)
            .take_while(|(sub, _raw)| sub.start < end)
            .inspect(|x| {
                trace!(">>> item {:?} ∈ {:?}", &condensed_range, x.0);
            })
            .filter(|(sub, _)| {
                // could possibly happen on empty documentation lines with `///`
                sub.len() > 0
            })
            .fold(IndexMap::<_, _>::new(), |mut acc, (sub, raw)| {
                fn recombine(range: Range, offset: usize, len: usize) -> Range {
                    Range {
                        start: range.start + offset,
                        end: range.start + offset + len,
                    }
                };
                let _ = if sub.contains(&start) {
                    // calculate the offset between our `condensed_range.start` and
                    // the `sub` which is one entry in the mappings
                    let offset = start - sub.start;
                    if sub.contains(&(end - 1)) {
                        // complete start to end
                        active = false;
                        let raw = recombine(raw.clone(), offset, end - start);
                        Some((start..end, raw))
                    } else {
                        // only start, continue taking until end
                        active = true;
                        let raw = recombine(raw.clone(), offset, sub.end - start);
                        Some((start..sub.end, raw))
                    }
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
                    trace!("convert:  cmark-erased={:?} -> raw={:?}", sub, raw);

                    if raw.len() > 0 {
                        let resolved = self.raw.find_spans(raw.clone());
                        trace!("cmark-erased range to spans: {:?} -> {:?}", raw, resolved);
                        acc.extend(resolved.into_iter());
                    } else {
                        warn!("linear range to spans: {:?} empty!", raw);
                    }
                    Some(())
                });
                acc
            })
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
            itertools::cons_tuples(itertools::zip(self.mapping.iter(), color_cycle))
        {
            // TODO do this properly, `saturating sub` just prevents crashing
            let delta = md_range.start.saturating_sub(previous_md_end);
            // take care of the markers and things that are not rendered
            if delta > 0 {
                let s = sub_chars(commonmark.as_str(), previous_md_end..md_range.start);
                coloured_md.push_str(uncovered.apply_to(s.as_str()).to_string().as_str());
            }
            previous_md_end = md_range.end;

            let s = sub_chars(commonmark.as_str(), md_range.clone());
            coloured_md.push_str(style.apply_to(s.as_str()).to_string().as_str());

            let s = sub_chars(self.plain.as_str(), plain_range.clone());
            coloured_plain.push_str(style.apply_to(s.as_str()).to_string().as_str());
        }
        // write!(formatter, "{}", coloured_md)?;

        writeln!(formatter, "Commonmark:\n{}", coloured_md)?;
        writeln!(formatter, "Plain:\n{}", coloured_plain)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drill_span() {
        const TEST: &str = r##"ab **🐡** xy"##;
        let chunk = CheckableChunk::from_str(
            TEST,
            indexmap::indexmap! { 0..11 => Span {
                start: LineColumn {
                    line: 1usize,
                    column: 4usize,
                },
                end: LineColumn {
                    line: 1usize,
                    column: 14usize,
                },
            }},
        );

        let plain = chunk.erase_markdown();
        assert_eq!(plain.find_spans(0..2).len(), 1);
        assert_eq!(plain.find_spans(3..4).len(), 1);
        assert_eq!(plain.find_spans(5..7).len(), 1);
        assert_eq!(plain.find_spans(5..12).len(), 1);
        assert_eq!(plain.find_spans(9..20).len(), 0);
    }

    #[test]
    fn reduction_complex() {
        const MARKDOWN: &str = r##"# Title number 1

## Title number 2

```rust
let x = 777;
let y = 111;
let z = x/y;
assert_eq!(z,7);
```

### Title [number 3][ff]

Some **extra** _formatting_ if __anticipated__ or _*not*_ or
maybe not at all.


Extra ~pagaph~ _paragraph_.

---

And a line, or a **rule**.


[ff]: https://docs.rs
"##;

        const PLAIN: &str = r##"Title number 1

Title number 2

Title number 3

Some extra formatting if anticipated or not or
maybe not at all.

Extra ~pagaph~ paragraph.


And a line, or a rule."##;
        let (reduced, mapping) = PlainOverlay::extract_plain_with_mapping(MARKDOWN);

        assert_eq!(dbg!(&reduced).as_str(), PLAIN);
        assert_eq!(dbg!(&mapping).len(), 19);
        for (reduced_range, cmark_range) in mapping.iter() {
            assert_eq!(
                reduced[reduced_range.clone()],
                MARKDOWN[cmark_range.clone()]
            );
        }
    }

    #[test]
    fn reduction_leading_space() {
        const MARKDOWN: &str = r#"  Some __underlined__ **bold** text."#;
        const PLAIN: &str = r#"Some underlined bold text."#;

        let (reduced, mapping) = PlainOverlay::extract_plain_with_mapping(MARKDOWN);

        assert_eq!(dbg!(&reduced).as_str(), PLAIN);
        assert_eq!(dbg!(&mapping).len(), 5);
        for (reduced_range, cmark_range) in mapping.iter() {
            assert_eq!(
                reduced[reduced_range.clone()].to_owned(),
                MARKDOWN[cmark_range.clone()].to_owned()
            );
        }
    }

    #[test]
    fn range_test() {
        let mut x = IndexMap::<Range, Range>::new();
        x.insert(0..2, 1..3);
        x.insert(3..4, 7..8);
        x.insert(5..12, 11..18);

        let lookmeup = 6..8;

        // TODO keep in sync with copy pasta source, extract a func for this
        let plain_range = lookmeup;
        let v: Vec<_> = x
            .iter()
            .filter(|(plain, _md)| plain.start <= plain_range.end && plain_range.start <= plain.end)
            .fold(Vec::with_capacity(64), |mut acc, (plain, md)| {
                // calculate the linear shift
                let offset = dbg!(md.start - plain.start);
                assert_eq!(md.end - plain.end, offset);
                let extracted = Range {
                    start: plain_range.start + offset,
                    end: core::cmp::min(md.end, plain_range.end + offset),
                };
                acc.push(extracted);
                acc
            });
        assert_eq!(v.first(), Some(&(12..14)));
    }

    fn cmark_reduction_test(
        input: &'static str,
        expected: &'static str,
        expected_mapping_len: usize,
    ) {
        let (plain, mapping) = PlainOverlay::extract_plain_with_mapping(input);
        assert_eq!(dbg!(&plain).as_str(), expected);
        assert_eq!(dbg!(&mapping).len(), expected_mapping_len);
        for (reduced_range, markdown_range) in mapping.iter() {
            assert_eq!(
                dbg!(crate::util::sub_chars(&plain, reduced_range.clone())),
                dbg!(crate::util::sub_chars(&input, markdown_range.clone()))
            );
        }
    }

    #[test]
    fn emoji() {
        cmark_reduction_test(
            r#"
Abcd

---

e🌡🍁

---

fgh"#,
            r#"Abcd


e🌡🍁


fgh"#,
            3,
        );
    }

    #[test]
    fn link_footnote() {
        cmark_reduction_test(
            r#"footnote [^linktxt]. Which one?

[linktxt]: ../../reference/index.html"#,
            r#"footnote linktxt. Which one?"#,
            3,
        );
    }

    #[test]
    fn link_inline() {
        cmark_reduction_test(
            r#" prefix [I'm an inline-style link](https://duckduckgo.com) postfix"#,
            r#"prefix I'm an inline-style link postfix"#,
            3,
        );
    }
    #[test]
    fn link_auto() {
        cmark_reduction_test(
            r#" prefix <http://foo.bar/baz> postfix"#,
            r#"prefix  postfix"#,
            2,
        );
        cmark_reduction_test(r#" <http://foo.bar/baz>"#, r#""#, 0);
    }

    #[test]
    fn link_email() {
        cmark_reduction_test(
            r#" prefix <loe@example.com> postfix"#,
            r#"prefix  postfix"#,
            2,
        );
    }

    #[test]
    fn link_reference() {
        cmark_reduction_test(
            r#"[classy reference link][the reference str]"#,
            r#"classy reference link"#,
            1,
        );
    }

    #[test]
    fn link_collapsed_ref() {
        cmark_reduction_test(
            r#"[collapsed reference link][]"#,
            r#"collapsed reference link"#,
            1,
        );
    }

    #[test]
    fn link_shortcut_ref() {
        cmark_reduction_test(
            r#"[shortcut reference link]"#,
            r#"shortcut reference link"#,
            1,
        );
    }
    // Nested links as well as nested code blocks are
    // impossible according to the common mark spec.

    #[test]
    fn list_nested() {
        cmark_reduction_test(
            r#"
* [x] a
* [ ] b
  * [ ] c
  * [x] d
"#,
            r#"
a
b
c
d"#,
            4,
        );
    }

    #[test]
    fn table_ignore() {
        // TODO FIXME it would be better to transform this into
        // one line per cell and test each cell.
        // TODO very most likely will cause issues with grammar checks
        // so eventually this will have to become checker specific code
        // or handle a list of mute tags to simply ignore.
        cmark_reduction_test(
            r#"
00

|a|b|c
|-|-|-
|p|q|r

ff
"#,
            r#"00


ff"#,
            2,
        );
    }
}
