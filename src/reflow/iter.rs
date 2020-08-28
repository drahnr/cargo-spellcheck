use super::*;

use std::borrow::Cow;
use std::collections::VecDeque;

/// Tokenizes a section which is delimited by untokenizable content.
pub struct Tokeneer<'s> {
    /// Original source string of continuous lines which are to be wrapped.
    s: &'s str,

    /// The sub-range of the string to reflow.
    range: Range,
    /// A set of matching spans to sub ranges.
    spans: IndexMap<Range, Span>,

    /// If there would occur a line break, that falls within a range of this
    /// the break would only occur afterwards or the whole word gets moved to
    /// the next line.
    /// MUST be sorted by `.start` value.
    /// Ranges must be in characters!
    unbreakable_ranges: Vec<Range>,
    unbreakable_idx: usize,

    inner: std::iter::Peekable<std::iter::Enumerate<std::str::CharIndices<'s>>>,
    // in bytes
    previous_byte_offset: usize,
    // in chars!
    previous_char_offset: usize,
}

impl<'s> Tokeneer<'s> {
    /// Initialize a new tokenizer
    pub fn new(s: &'s str, range: Range) -> Self {
        let inner = s.char_indices().enumerate().peekable();
        Self {
            s,
            range,
            spans: Default::default(),
            unbreakable_ranges: Vec::new(),
            unbreakable_idx: 0usize,
            inner,
            previous_byte_offset: 0usize,
            previous_char_offset: 0usize,
        }
    }

    #[inline(always)]
    pub fn add_unbreakables(&mut self, unbreakable_ranges: impl IntoIterator<Item = Range>) {
        self.unbreakable_ranges.extend(unbreakable_ranges);
    }
}

impl<'s> Iterator for Tokeneer<'s> {
    /// `(char range, byte range, )`
    type Item = (Range, Range, Cow<'s, str>);
    /// Yields individual tokens with associated ranges in chars and bytes.
    fn next(&mut self) -> Option<Self::Item> {
        'outer: while let Some((char_idx, (byte_offset, c))) = self.inner.next() {
            'unbreakable: while let Some(unbreakable) =
                self.unbreakable_ranges.get(self.unbreakable_idx)
            {
                // are we within an unbreakable area
                // iff so, just continue until we are out of the weeds
                if unbreakable.contains(&char_idx) {
                    continue 'outer;
                }
                let idx = self.unbreakable_idx + 1;
                if let Some(unbreakable_next) = self.unbreakable_ranges.get(idx) {
                    // check if the next unbreakable is overlapping, should not, but hey
                    if unbreakable_next.contains(&char_idx) {
                        // if yes, adjust the index and keep going
                        self.unbreakable_idx = idx;
                        continue 'unbreakable;
                    }
                } else {
                    break 'unbreakable;
                }
                // we are out of the weeds, the next round needs the next index
                self.unbreakable_idx = idx;
            }

            if c.is_whitespace() {
                // track the last whitespace, that plus one marks the start of the next word
                self.previous_byte_offset = byte_offset + 1;
                self.previous_char_offset = char_idx + 1;
                continue 'outer;
            }
            let byte_range = if let Some(&(_char_idx, (byte_offset_next, c_next))) =
                self.inner.peek()
            {
                if !c_next.is_whitespace() {
                    // we have a non whitespace char ahead, so this is not a breaking position
                    continue 'outer;
                }
                // if the next peek char is a whitespace, that means we reached the end of the word
                self.previous_byte_offset..byte_offset_next
            } else if self.previous_byte_offset < byte_offset {
                // peek is None, so we might be at the end of the string
                // so we return the last part
                self.previous_byte_offset..byte_offset
            } else if cfg!(debug_assertions) {
                unreachable!("Unreachable condition was reached");
            } else {
                log::error!("Should be never be reachable, please file a bug with the corresponding input data");
                break 'outer;
            };

            let char_range = self.previous_char_offset..char_idx + 1;

            let item = (
                char_range.clone(),
                byte_range.clone(),
                std::borrow::Cow::Borrowed(&self.s[byte_range]),
            );
            return Some(item);
        }
        None
    }
}

/// Re-glue all tokenized items under the constrained of a maximum line width
pub struct Gluon<'s> {
    /// Stores a sequence of undividable items as `(char range, cow str)`,
    /// which are eventually combined to a new line.
    queue: VecDeque<(Range, std::borrow::Cow<'s, str>)>,
    /// Maximum width in columns to be used before breaking a line.
    max_line_width: usize,
    /// Internal counter for keeping track of the line index with base 1.
    line_counter: usize,
    /// A set of indentations for considering indentations
    /// if there are more lines than there are lines, the last
    /// values in this vector willl bre reused.
    indentations: Vec<usize>,
    /// The inner iterator which first tokenizes the string into undividable items.
    inner: Tokeneer<'s>,
}

impl<'s> Gluon<'s> {
    pub fn new(s: &'s str, range: Range, max_line_width: usize, indentations: Vec<usize>) -> Self {
        Self {
            queue: VecDeque::new(),
            max_line_width,
            indentations,
            line_counter: 0usize,
            inner: Tokeneer::<'s>::new(s, range),
        }
    }

    #[inline(always)]
    #[allow(unused)]
    pub fn add_unbreakables(&mut self, unbreakable_ranges: impl IntoIterator<Item = Range>) {
        self.inner.add_unbreakables(unbreakable_ranges);
    }

    /// Create a new line based on the processing queue
    fn craft_line(&mut self) -> (usize, String, Range) {
        use itertools::Itertools;
        self.line_counter += 1;
        let mut char_range = usize::MAX..0;
        let line_content = self
            .queue
            .drain(..)
            .map(|(range, s)| {
                char_range.start = std::cmp::min(range.start, char_range.start);
                char_range.end = std::cmp::max(range.end, char_range.end);
                s
            })
            .join(" ");
        (self.line_counter, line_content, char_range)
    }
}

impl<'s> Iterator for Gluon<'s> {
    /// Returns a tuple of `(line number, line content, char range)`.
    type Item = (usize, String, Range);

    fn next(&mut self) -> Option<Self::Item> {
        // obtain the indentation level for this line
        // we do keep the indentation unless there are
        // no sufficient lines which causes us to use
        // the last used one
        // Iff the whole thing was empty to begin with, `0`
        // is the default fallback.
        let offset = self
            .indentations
            .get(
                // the next not yet crafted line
                self.line_counter + 1,
            )
            .map(|x: &usize| *x)
            .unwrap_or_else(|| {
                self.indentations
                    .last()
                    .map(|x: &usize| *x)
                    .unwrap_or(0usize)
            });

        while let Some((char_range, _byte_range, cow_str)) = self.inner.next() {
            // calculate the current characters that are already in that line
            // and assume one whitespace inbetween each of them
            let mut qiter = self.queue.iter();
            let acc_len = if let Some((first, _)) = qiter.next() {
                qiter
                    .map(|(r, _)| r.len())
                    .fold(first.len(), |acc, len| acc + 1 + len)
            } else {
                0usize
            };

            let item_len = char_range.len();
            let item = (char_range.clone(), cow_str);
            let ret = if offset + acc_len <= self.max_line_width {
                // calculate the sum if we would add the word
                let sum = offset + acc_len + 1 + item_len;
                if sum > self.max_line_width {
                    // if the line length would be exceeded
                    let ret = self.craft_line();
                    self.queue.push_back(item);
                    ret
                } else {
                    self.queue.push_back(item);
                    continue;
                }
            } else if item_len > self.max_line_width {
                log::warn!(
                    "A unbreakable chunk is larger than the max line width {} vs {}",
                    item_len,
                    self.max_line_width
                );
                if acc_len > 0 {
                    // craft a line before inserting
                    // to avoid exceeding too much
                    // so use what is in there now and leave the overly long item for the next `next()` call
                    let line = self.craft_line();
                    self.queue.push_back(item);
                    line
                } else {
                    self.queue.push_back(item);
                    continue;
                }
            } else {
                // the queue len is already larger than what the max line length allows
                // since we do this iteratively, it should only happen for an unbreakable statement
                // that exceeds the max line length or very large offsets
                let ret = self.craft_line();
                self.queue.push_back(item);
                ret
            };
            return Some(ret);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `indentations` represent the indentation of indiviaul lines in `content`.
    fn verify_reflow(
        content: &'static str,
        expected: &'static str,
        max_line_width: usize,
        unbreakables: Vec<Range>,
        indentations: Vec<usize>,
    ) {
        let expected = expected
            .lines()
            .enumerate()
            .map(|(idx, line)| (idx + 1, line));
        let mut gluon = Gluon::new(content, 0..content.len(), max_line_width, indentations);

        gluon.add_unbreakables(unbreakables);

        for ((line_no, line_content, _), (expected_no, expected_content)) in gluon.zip(expected) {
            assert_eq!(line_no, expected_no);
            assert_eq!(dbg!(line_content), dbg!(expected_content));
        }
    }

    #[test]
    fn wrap_too_long_fluid() {
        const CONTENT: &'static str = "something kinda too long for a single line";
        const EXPECTED: &'static str = r#"something kinda too long for a
 single line"#;
        verify_reflow(CONTENT, EXPECTED, 30usize, vec![], vec![0]);
    }

    #[test]
    fn wrap_too_short_fluid() {
        const CONTENT: &'static str = r#"something
kinda
too
short
for
a
single
line"#;

        const EXPECTED: &'static str = r#"something kinda too short for
a single line"#;

        verify_reflow(CONTENT, EXPECTED, 30usize, vec![], vec![0; 8]);
    }

    #[test]
    fn wrap_too_long_unbreakable() {
        const CONTENT: &'static str = "something kinda too Xong for a singlX line";
        const EXPECTED: &'static str = r#"something kinda too
Xong for a singlX line"#;
        verify_reflow(CONTENT, EXPECTED, 30usize, vec![20..37], vec![0]);
    }

    #[test]
    fn spaces_and_tabs() {
        const CONTENT: &'static str = "        something     kinda       ";
        const EXPECTED: &'static str = r#"something
kinda"#;
        verify_reflow(CONTENT, EXPECTED, 20usize, vec![], vec![0]);
    }

    #[test]
    fn deep_indentation_too_long() {
        const CONTENT: &'static str = r#"deep indentation"#;
        const EXPECTED: &'static str = r#"deep
indentation"#;
        // 20 < 31 = 4 + 1 + 11 + 15
        verify_reflow(CONTENT, EXPECTED, 20usize, vec![], vec![15]);
    }

    #[test]
    fn deep_indentation_too_short() {
        let _ = env_logger::builder()
            .is_test(true)
            .filter(None, log::LevelFilter::Trace)
            .try_init();

        const CONTENT: &'static str = r#"deep
indentation"#;
        const EXPECTED: &'static str = r#"deep indentation"#;
        // 22 > 21 = 4 + 1 + 11 + 5
        verify_reflow(CONTENT, EXPECTED, 22usize, vec![], vec![5, 5]);
    }
}
