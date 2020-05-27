//! Erase markdown syntax
//!
//! Resulting overlay is plain and can be fed into a grammer or spell checker.

use super::*;
use crate::{LineColumn, Span};

use log::trace;
use pulldown_cmark::{Event, Options, Parser, Tag};

use crate::literalset::LiteralSet;
use proc_macro2::Literal;

use indexmap::IndexMap;

pub type Range = core::ops::Range<usize>;

/// A plain representation of markdown riddled set of trimmed literals.
#[derive(Clone)]
pub struct PlainOverlay<'a> {
    raw: &'a LiteralSet,
    plain: String,
    // require a sorted map, so we have the chance of binary search
    // key: plain string range
    // value: the corresponding areas in the full markdown
    mapping: IndexMap<Range, Range>,
}

impl<'a> PlainOverlay<'a> {
    fn track(s: &str, offset: Range, plain: &mut String, mapping: &mut IndexMap<Range, Range>) {
        // map the range within the plain data,
        // which is fed to the checker,
        // back to the repr with markdown modifiers
        let _ = mapping.insert(
            Range {
                start: plain.len(),
                end: plain.len() + s.len(),
            },
            offset,
        );
        plain.push_str(&s);
    }

    fn newline(plain: &mut String) {
        plain.push('\n');
    }

    fn newlines(plain: &mut String, n: usize) {
        for _ in 0..n {
            plain.push('\n');
        }
    }

    fn extract_plain_with_mapping(markdown: &str) -> (String, IndexMap<Range, Range>) {
        let mut plain = String::with_capacity(markdown.len());
        let mut mapping = indexmap::IndexMap::with_capacity(128);

        let parser = Parser::new_ext(markdown, Options::all());

        let rust_fence = pulldown_cmark::CowStr::Borrowed("rust");

        let mut code_block = false;
        for (event, offset) in parser.into_offset_iter() {
            match event {
                Event::Start(tag) => {
                    // @todo check links
                    match dbg!(tag) {
                        Tag::CodeBlock(fenced) => {
                            code_block = true;

                            match fenced {
                                pulldown_cmark::CodeBlockKind::Fenced(rust_fence) => {
                                    // @todo validate this as an extra document entity
                                }
                                _ => {}
                            }
                        }

                        _ => {}
                    }
                }
                Event::End(tag) => {
                    match dbg!(tag) {
                        Tag::Link(link_type, url, title) => {
                            // @todo check links
                            Self::track(&title, offset, &mut plain, &mut mapping);
                        }
                        Tag::Image(link_type, url, title) => {
                            Self::track(&title, offset, &mut plain, &mut mapping);
                        }
                        Tag::Heading(n) => {
                            Self::newlines(&mut plain, 2);
                        }
                        Tag::CodeBlock(fenced) => {
                            code_block = false;

                            match fenced {
                                pulldown_cmark::CodeBlockKind::Fenced(rust_fence) => {
                                    // @todo validate this as an extra document entity
                                }
                                _ => {}
                            }
                        }
                        Tag::Paragraph => Self::newlines(&mut plain, 2),
                        _ => {}
                    }
                }
                Event::Text(s) => {
                    if code_block {
                        let _ = dbg!(&s);
                    } else {
                        Self::track(dbg!(&s), offset, &mut plain, &mut mapping);
                    }
                }
                Event::Code(s) => {
                    // @todo extract comments from the doc comment and in the distant
                    // future potentially also check var names with leviatan distance
                    // to wordbook entries, and only complain if there are sane suggestions
                }
                Event::Html(s) => {}
                Event::FootnoteReference(s) => {
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
                Event::TaskListMarker(b) => {}
            }
        }
        (plain, mapping)
    }

    // @todo consider returning a Vec<PlainOverlay<'a>> to account for list items
    // or other chunked information which might not pass a grammar check as a whole
    pub fn erase_markdown(literal_set: &'a LiteralSet) -> Self {
        let markdown = literal_set.to_string();

        let (plain, mapping) = Self::extract_plain_with_mapping(markdown.as_str());
        Self {
            raw: literal_set,
            plain,
            mapping,
        }
    }

    pub fn linear_range_to_spans(&self, plain_range: Range) -> Vec<(&'a TrimmedLiteral, Span)> {
        // check for the start index in range
        self.mapping
            .iter()
            .skip_while(|(plain, _md)| plain.end <= plain_range.start)
            .take_while(|(plain, _md)| plain.end < plain_range.end)
            .fold(Vec::with_capacity(64), |mut acc, (plain, md)| {
                acc.extend(self.raw.linear_range_to_spans(md.clone()).into_iter());
                acc
            })
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
        use itertools;

        let styles = vec![
            Style::new().underlined().bold().dim().red(),
            Style::new().underlined().bold().dim().green(),
            Style::new().underlined().bold().dim().yellow(),
            Style::new().underlined().bold().dim().magenta(),
			Style::new().underlined().bold().dim().cyan(),
        ];

		let uncovered = Style::new().bold().on_black().dim().white();

        let color_cycle = styles.iter().cycle();


		let markdown = self.raw.to_string();

		let mut coloured_plain = String::with_capacity(1024);
		let mut coloured_md = String::with_capacity(1024);

		let mut previous_md_end = 0usize;
        for (plain_range, md_range, style) in itertools::cons_tuples(itertools::zip(self.mapping.iter(), color_cycle)) {
			let delta = md_range.start - previous_md_end;
			// take care of the markers and things that are not rendered
			if delta > 0 {
				coloured_md.push_str(uncovered.apply_to(&markdown[previous_md_end..md_range.start]).to_string().as_str());
			}
			previous_md_end = md_range.end;

			coloured_md.push_str(style.apply_to(&markdown[md_range.clone()]).to_string().as_str());

			// coloured_plain.push_str(style.apply_to(&self.plain[plain_range.clone()]).to_string().as_str());
		}
		write!(formatter, "{}", coloured_md)?;

		// write!(formatter, "Plain:\n{}", coloured_plain)?;
		// write!(formatter, "Markdown:\n{}", coloured_md)?;
		Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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


And a line, or a rule.

"##;
    #[test]
    fn markdown_to_plain_with_mapping() {
        let (plain, mapping) = PlainOverlay::extract_plain_with_mapping(MARKDOWN);

        assert_eq!(plain.as_str(), PLAIN);
        assert_eq!(dbg!(mapping).len(), 19);
    }
}
