//! Erase markdown syntax
//!
//! Resulting overlay is plain and can be fed into a grammer or spell checker.

use super::*;

use indexmap::IndexMap;
use log::trace;
use pulldown_cmark::{Event, Options, Parser, Tag};

use crate::documentation::{CheckableChunk, Range};
use crate::Span;

/// A plain representation of markdown riddled set of trimmed literals.
#[derive(Clone)]
pub struct PlainOverlay<'a> {
    raw: &'a CheckableChunk,
    plain: String,
    // require a sorted map, so we have the chance of binary search
    // key: plain string range
    // value: the corresponding areas in the full markdown
    mapping: IndexMap<Range, Range>,
}

impl<'a> PlainOverlay<'a> {
    fn track(s: &str, markdown: Range, plain: &mut String, mapping: &mut IndexMap<Range, Range>) {
        // map the range within the plain data,
        // which is fed to the checker,
        // back to the repr with markdown modifiers
        let _ = mapping.insert(
            Range {
                start: plain.len(),
                end: plain.len() + s.len(),
            },
            markdown,
        );
        plain.push_str(&s);
    }

    fn newlines(plain: &mut String, n: usize) {
        for _ in 0..n {
            plain.push('\n');
        }
    }

    /// ranges are mapped `cmakr reduced/plain -> raw`
    fn extract_plain_with_mapping(markdown: &str) -> (String, IndexMap<Range, Range>) {
        let mut plain = String::with_capacity(markdown.len());
        let mut mapping = indexmap::IndexMap::with_capacity(128);

        let parser = Parser::new_ext(markdown, Options::all());

        let rust_fence =
            pulldown_cmark::CodeBlockKind::Fenced(pulldown_cmark::CowStr::Borrowed("rust"));

        let mut code_block = false;

        for (event, offset) in parser.into_offset_iter() {
            trace!("Parsing event ({:?}): {:?}", &offset, &event);
            match event {
                Event::Start(tag) => {
                    // @todo check links
                    match tag {
                        Tag::CodeBlock(fenced) => {
                            code_block = true;

                            if fenced == rust_fence {
                                // @todo validate as if it was another document entity
                            }
                        }

                        _ => {}
                    }
                }
                Event::End(tag) => {
                    match tag {
                        Tag::Link(_link_type, _url, title) => {
                            // @todo check links
                            Self::track(&title, offset, &mut plain, &mut mapping);
                        }
                        Tag::Image(_link_type, _url, title) => {
                            Self::track(&title, offset, &mut plain, &mut mapping);
                        }
                        Tag::Heading(_n) => {
                            Self::newlines(&mut plain, 2);
                        }
                        Tag::CodeBlock(fenced) => {
                            code_block = false;

                            if fenced == rust_fence {
                                // @todo validate as if it was another document entity
                            }
                        }
                        Tag::Paragraph => Self::newlines(&mut plain, 2),
                        _ => {}
                    }
                }
                Event::Text(s) => {
                    if code_block {
                    } else {
                        Self::track(&s, offset, &mut plain, &mut mapping);
                    }
                }
                Event::Code(_s) => {
                    // @todo extract comments from the doc comment and in the distant
                    // future potentially also check var names with leviatan distance
                    // to wordbook entries, and only complain if there are sane suggestions
                }
                Event::Html(_s) => {}
                Event::FootnoteReference(_s) => {
                    // @todo handle footnotes
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
                Event::TaskListMarker(_b) => {}
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

    // @todo consider returning a Vec<PlainOverlay<'a>> to account for list items
    // or other non-linear information which might not pass a grammar check as a whole
    pub fn erase_markdown(chunk: &'a CheckableChunk) -> Self {
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
            .take_while(|(sub, _raw)| end <= sub.end)
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
                // @todo must be implemented properly
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

        let markdown = self.raw.as_str().to_owned();

        let mut coloured_plain = String::with_capacity(1024);
        let mut coloured_md = String::with_capacity(1024);

        let mut previous_md_end = 0usize;
        for (_plain_range, md_range, style) in
            itertools::cons_tuples(itertools::zip(self.mapping.iter(), color_cycle))
        {
            let delta = md_range.start - previous_md_end;
            // take care of the markers and things that are not rendered
            if delta > 0 {
                coloured_md.push_str(
                    uncovered
                        .apply_to(&markdown[previous_md_end..md_range.start])
                        .to_string()
                        .as_str(),
                );
            }
            previous_md_end = md_range.end;

            coloured_md.push_str(
                style
                    .apply_to(&markdown[md_range.clone()])
                    .to_string()
                    .as_str(),
            );

            coloured_plain.push_str(
                style
                    .apply_to(&self.plain[_plain_range.clone()])
                    .to_string()
                    .as_str(),
            );
        }
        // write!(formatter, "{}", coloured_md)?;

        writeln!(formatter, "Markdown:\n{}", coloured_md)?;
        writeln!(formatter, "Plain:\n{}", coloured_plain)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_reduction_mapping() {
        // @todo add links
        const MARKDOWN: &str = r##"# Title number 1

## Title number 2

```rust
let x = 777;
let y = 111;
let z = x/y;
assert_eq!(z,7);
```

### Title number 3

Some **extra** _formatting_ if __anticipated__ or _*not*_ or
maybe not at all.


Extra ~pagaph~ _paragraph_.

---

And a line, or a **rule**.

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
        for (reduced_range, markdown_range) in mapping.iter() {
            assert_eq!(
                reduced[reduced_range.clone()],
                MARKDOWN[markdown_range.clone()]
            );
        }
    }

    #[test]
    fn markdown_reduction_mapping_leading_space() {
        const MARKDOWN: &str = r#"  Some __underlined__ **bold** text."#;
        const PLAIN: &str = r#"Some underlined bold text."#;

        let (reduced, mapping) = PlainOverlay::extract_plain_with_mapping(MARKDOWN);

        assert_eq!(dbg!(&reduced).as_str(), PLAIN);
        assert_eq!(dbg!(&mapping).len(), 5);
        for (reduced_range, markdown_range) in mapping.iter() {
            assert_eq!(
                reduced[reduced_range.clone()].to_owned(),
                MARKDOWN[markdown_range.clone()].to_owned()
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

        // @todo keep in sync with copy pasta source, extract a func for this
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
}
