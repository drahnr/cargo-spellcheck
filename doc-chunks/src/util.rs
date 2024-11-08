use crate::errors::*;
use crate::{LineColumn, Range, Span};
use core::ops::{Bound, RangeBounds};
use fs_err as fs;
use std::io::Read;
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
struct LineSepStat {
    first_appearance: usize,
    count: usize,
    newline: &'static str,
}

#[inline(always)]
fn extract_delimiter_inner<'a>(
    mut iter: impl Iterator<Item = usize>,
    newline: &'static str,
) -> Option<LineSepStat> {
    if let Some(first) = iter.next() {
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
pub fn extract_delimiter(s: &str) -> Option<&'static str> {
    // TODO lots of room for optimizations here
    let lf = memchr::memchr_iter(b'\n', s.as_bytes());
    let cr = memchr::memchr_iter(b'\r', s.as_bytes());
    let crlf = memchr::memmem::find_iter(s.as_bytes(), "\r\n");
    let lfcr = memchr::memmem::find_iter(s.as_bytes(), "\n\r");
    // first look for two letter line delimiters
    let lfcr = extract_delimiter_inner(lfcr, "\n\r");
    let crlf = extract_delimiter_inner(crlf, "\r\n");

    // remove the 2 line line delimiters from the single line line delimiters, since they overlap
    let lf = extract_delimiter_inner(lf, "\n").map(|mut stat| {
        stat.count = stat.count.saturating_sub(std::cmp::max(
            crlf.as_ref().map(|stat| stat.count).unwrap_or_default(),
            lfcr.as_ref().map(|stat| stat.count).unwrap_or_default(),
        ));
        stat
    });
    let cr = extract_delimiter_inner(cr, "\r").map(|mut stat| {
        stat.count = stat.count.saturating_sub(std::cmp::max(
            crlf.as_ref().map(|stat| stat.count).unwrap_or_default(),
            lfcr.as_ref().map(|stat| stat.count).unwrap_or_default(),
        ));
        stat
    });

    // order is important, `max_by` prefers the latter ones over the earlier ones on equality
    vec![cr, lf, crlf, lfcr]
        .into_iter()
        .flatten()
        .max_by(|b, a| {
            if a.count == b.count {
                a.first_appearance.cmp(&b.first_appearance)
            } else {
                b.count.cmp(&a.count)
            }
        })
        .map(|x| x.newline)
}

/// Iterate over a str and annotate with line and column.
///
/// Assumes `s` is content starting from point `start_point`.
pub fn iter_with_line_column_from(
    s: &str,
    start_point: LineColumn,
) -> impl Iterator<Item = (char, usize, usize, LineColumn)> + '_ {
    #[derive(Clone)]
    struct State {
        cursor: LineColumn,
        previous_char_was_newline: bool,
    }

    let initial = State {
        cursor: start_point,
        previous_char_was_newline: false,
    };

    s.char_indices()
        .enumerate()
        .map(|(idx, (byte_offset, c))| (idx, byte_offset, c))
        .scan(initial, |state, (idx, byte_offset, c)| -> Option<_> {
            let cursor = state.cursor;
            state.previous_char_was_newline = c == '\n';
            if state.previous_char_was_newline {
                state.cursor.line += 1;
                state.cursor.column = 0;
            } else {
                state.cursor.column += 1;
            }
            Some((c, byte_offset, idx, cursor))
        })
}

/// Iterate over annotated chars starting from line 1 and column 0 assuming `s`
/// starts there.
pub fn iter_with_line_column(
    s: &str,
) -> impl Iterator<Item = (char, usize, usize, LineColumn)> + '_ {
    iter_with_line_column_from(s, LineColumn { line: 1, column: 0 })
}

/// Extract `span` from a `Read`-able source as `String`.
///
/// # Errors
/// Returns an Error if `span` describes a impossible range.
pub fn load_span_from<R>(mut source: R, span: Span) -> Result<String>
where
    R: Read,
{
    log::trace!("Loading {span:?} from source");
    if span.start.line < 1 {
        return Err(Error::Span(
            "Lines are 1-indexed, can't be less than 1".to_string(),
        ));
    }
    if span.end.line < span.start.line {
        return Err(Error::Span(
            "Line range would be negative, bail".to_string(),
        ));
    }
    if span.end.line == span.start.line && span.end.column < span.start.column {
        return Err(Error::Span(
            "Column range would be negative, bail".to_string(),
        ));
    }
    let mut s = String::with_capacity(256);
    source
        .read_to_string(&mut s)
        .expect("Must read successfully");

    let extraction = iter_with_line_column(s.as_str())
        .skip_while(|(_c, _byte_offset, _idx, cursor)| {
            cursor.line < span.start.line
                || (cursor.line == span.start.line && cursor.column < span.start.column)
        })
        .take_while(|(_c, _byte_offset, _idx, cursor)| {
            cursor.line < span.end.line
                || (cursor.line == span.end.line && cursor.column <= span.end.column)
        })
        .fuse()
        .map(|(c, _byte_offset, _idx, _cursor)| c)
        .collect::<String>();
    // log::trace!("Loading {range:?} from line >{line}<");
    Ok(extraction)
}

/// Extract span from a file as `String`.
///
/// Helpful to validate bandaids against what's actually in the file.
#[allow(unused)]
pub(crate) fn load_span_from_file(path: impl AsRef<Path>, span: Span) -> Result<String> {
    let path = path.as_ref();
    let path = fs::canonicalize(path)?;

    let ro = fs::OpenOptions::new().read(true).open(&path)?;

    let mut reader = std::io::BufReader::new(ro);

    load_span_from(reader, span)
}

/// Extract a subset of chars by iterating. Range must be in characters.
pub fn sub_chars(s: &str, range: Range) -> String {
    s.chars()
        .skip(range.start)
        .take(range.len())
        .collect::<String>()
}

/// Convert a given byte range of a string, that is known to be at valid char
/// bounds, to a character range.
///
/// If the bounds are not bounded by a character, it will take the bounding
/// characters that are intersected inclusive.
pub fn byte_range_to_char_range<R>(s: &str, byte_range: R) -> Option<Range>
where
    R: RangeBounds<usize>,
{
    let mut peekable = s.char_indices().enumerate().peekable();
    let mut range = Range { start: 0, end: 0 };
    let mut started = false;
    while let Some((idx, (byte_offset, _c))) = peekable.next() {
        match byte_range.start_bound() {
            Bound::Included(&start) if byte_offset == start => {
                started = true;
                range.start = idx;
            }
            Bound::Included(&start) if byte_offset > start && !started => {
                started = true;
                range.start = idx.saturating_sub(1);
            }
            Bound::Excluded(_start) => {
                unreachable!("Exclusive start bounds do not exist. qed");
            }
            _ => {}
        }

        match byte_range.end_bound() {
            Bound::Included(&end) if byte_offset > end => {
                range.end = idx;
                return Some(range);
            }
            Bound::Excluded(&end) if byte_offset >= end => {
                range.end = idx;
                return Some(range);
            }
            _ => {}
        }
        if peekable.peek().is_none() && started {
            range.end = idx + 1;
            return Some(range);
        }
    }
    None
}

/// Convert many byte ranges to character ranges.
///
/// Attention: All byte ranges most NOT overlap and be relative to the same `s`.
pub fn byte_range_to_char_range_many<R>(s: &str, byte_ranges: &[R]) -> Vec<Range>
where
    R: std::ops::RangeBounds<usize> + std::fmt::Debug,
{
    let mut peekable = s.char_indices().enumerate().peekable();
    let mut cursor = 0usize;
    let mut acc = Vec::with_capacity(byte_ranges.len());
    for byte_range in byte_ranges {
        let mut range = Range { start: 0, end: 0 };
        let mut started = false;
        'inner: while let Some((idx, (byte_offset, _c))) = peekable.peek() {
            cursor = *idx;
            let byte_offset = *byte_offset;
            match byte_range.start_bound() {
                Bound::Included(&start) if byte_offset == start => {
                    started = true;
                    range.start = cursor;
                }
                Bound::Included(&start) if byte_offset > start && !started => {
                    started = true;
                    range.start = cursor.saturating_sub(1);
                }
                Bound::Excluded(_start) => {
                    unreachable!("Exclusive start bounds do not exist. qed");
                }
                _ => {}
            }

            match byte_range.end_bound() {
                Bound::Included(&end) if byte_offset > end => {
                    range.end = cursor;
                    acc.push(range.clone());
                    started = false;
                    break 'inner;
                }
                Bound::Excluded(&end) if byte_offset >= end => {
                    range.end = cursor;
                    acc.push(range.clone());
                    started = false;
                    break 'inner;
                }
                _ => {}
            }

            let _ = peekable.next();
        }
        if started {
            range.end = cursor + 1;
            acc.push(range);
        }
    }
    acc
}

/// Extract a subset of chars by iterating. Range must be in characters.
pub fn sub_char_range<R>(s: &str, range: R) -> &str
where
    R: RangeBounds<usize>,
{
    let mut peekable = s.char_indices().enumerate().peekable();
    let mut byte_range = Range { start: 0, end: 0 };
    let mut started = false;
    'loopy: while let Some((idx, (byte_offset_start, _c))) = peekable.next() {
        match range.start_bound() {
            Bound::Included(&start) if idx == start => {
                started = true;
                byte_range.start = byte_offset_start;
            }
            Bound::Excluded(_start) => {
                unreachable!("Exclusive start bounds do not exist. qed");
            }
            _ => {}
        }

        match range.end_bound() {
            Bound::Included(&end) if idx > end => {
                byte_range.end = byte_offset_start;
                break 'loopy;
            }
            Bound::Excluded(&end) if idx >= end => {
                byte_range.end = byte_offset_start;
                break 'loopy;
            }
            _ => {}
        }
        if peekable.peek().is_none() && started {
            byte_range.end = s.len();
        }
    }
    &s[byte_range]
}

#[cfg(test)]
mod tests {
    use super::*;
    macro_rules! lcc {
        ($line:literal, $column:literal, $c:literal) => {
            (
                LineColumn {
                    line: $line,
                    column: $column,
                },
                $c,
            )
        };
    }
    #[test]
    fn iter_chars() {
        const S: &str = r#"
abc
d
"#;
        const S2: &str = r#"c
d"#;
        const EXPECT: &[(LineColumn, char)] = &[
            lcc!(1, 0, '\n'),
            lcc!(2, 0, 'a'),
            lcc!(2, 1, 'b'),
            lcc!(2, 2, 'c'),
            lcc!(2, 3, '\n'),
            lcc!(3, 0, 'd'),
            lcc!(3, 1, '\n'),
        ];

        iter_with_line_column(S).zip(EXPECT.iter()).for_each(
            |((c, _byte_offset, _idx, lc), (expected_lc, expected_c))| {
                assert_eq!(lc, expected_lc.clone());
                assert_eq!(c, expected_c.clone());
            },
        );

        const SPAN: Span = Span {
            start: LineColumn { line: 2, column: 2 },
            end: LineColumn { line: 3, column: 0 },
        };

        assert_eq!(
            load_span_from(&mut S.as_bytes(), SPAN).expect("Must succeed"),
            S2.to_owned()
        );
    }

    #[test]
    fn iter_span_doc_0_trivial() {
        const SOURCE: &str = r##"#[doc=r#"Zebra
Schlupfwespe,
Grünfink"#]"##;
        const S2: &str = r#"Zebra
Schlupfwespe,
Grünfink"#;

        const SPAN: Span = Span {
            start: LineColumn {
                line: 1,
                column: 0 + 9,
            }, // prefix is #[doc=r#"
            end: LineColumn { line: 3, column: 7 }, // suffix is pointeless
        };

        assert_eq!(
            load_span_from(&mut SOURCE.as_bytes(), SPAN).expect("Must succeed"),
            S2.to_owned()
        );
    }

    #[test]
    fn iter_span_doc_1_trailing_newline() {
        const SOURCE: &str = r##"#[doc=r#"Zebra
Schlupfwespe,
"#]"##;
        const S2: &str = r#"Zebra
Schlupfwespe,
"#;

        const SPAN: Span = Span {
            start: LineColumn {
                line: 1,
                column: 0 + 9,
            }, // prefix is #[doc=r#"
            end: LineColumn {
                line: 2,
                column: 13,
            }, // suffix is pointeless
        };

        assert_eq!(
            load_span_from(&mut SOURCE.as_bytes(), SPAN).expect("Must succeed"),
            S2.to_owned()
        );
    }

    #[test]
    fn sub_a() {
        const A: &str = "a🐲o🌡i🡴f🕧aodnferntkng";
        const A_EXPECTED: &str = "a🐲o";

        assert_eq!(sub_char_range(A, 0..3), A_EXPECTED);
        assert_eq!(sub_char_range(A, ..3), A_EXPECTED);
        assert_eq!(sub_chars(A, 0..3), A_EXPECTED.to_owned());
    }

    #[test]
    fn sub_b() {
        const B: &str = "fff🦦🡴🕧";
        const B_EXPECTED: &str = "🦦🡴🕧";

        assert_eq!(sub_char_range(B, 3..=5), B_EXPECTED);
        assert_eq!(sub_char_range(B, 3..), B_EXPECTED);
    }

    #[test]
    fn sub_c() {
        const B: &str = "fff🦦🡴🕧";
        const B_EXPECTED: &str = "";

        assert_eq!(sub_char_range(B, 10..), B_EXPECTED);
        assert_eq!(sub_char_range(B, 15..16), B_EXPECTED);
    }

    #[test]
    fn range_bytes_to_chars() {
        // 4 3 4
        assert_eq!(byte_range_to_char_range("🕱™🐡", 4..7), Some(1..2));
        // 4 1 1 3 4
        assert_eq!(byte_range_to_char_range("🕱12™🐡", 6..13), Some(3..5));
        assert_eq!(byte_range_to_char_range("🕱12™🐡", 0..0), Some(0..0));
        assert_eq!(byte_range_to_char_range("🕱12™🐡", 25..26), None);
    }

    #[test]
    fn range_bytes_to_chars_many() {
        // 4 3 4
        assert_eq!(
            byte_range_to_char_range_many("🕱™🐡", &[4..7, 7..11]),
            vec![1..2, 2..3]
        );
        assert_eq!(
            byte_range_to_char_range_many("🕱™🐡", &[0..0, 4..11]),
            vec![0..0, 1..3]
        );
    }
}
