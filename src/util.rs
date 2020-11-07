use crate::{LineColumn, Range, Span};
use anyhow::{anyhow, bail, Result};
use std::io::Read;
use std::path::Path;

/// Iterate over a str and annotate with line and column.
///
/// Assumes `s` is content starting from point `start_point`.
pub fn iter_with_line_column_from<'a>(
    s: &'a str,
    start_point: LineColumn,
) -> impl Iterator<Item = (char, usize, usize, LineColumn)> + 'a {
    #[derive(Clone)]
    struct State {
        cursor: LineColumn,
        previous_char_was_newline: bool,
    };
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

/// Iterate over annotated chars starting from line 1 and column 0 assuming `s` starts there.
pub fn iter_with_line_column<'a>(
    s: &'a str,
) -> impl Iterator<Item = (char, usize, usize, LineColumn)> + 'a {
    iter_with_line_column_from(s, LineColumn { line: 1, column: 0 })
}

/// Extract `span` from a `Read`-able source as `String`
///
/// # Errors
/// Returns an Error if `span` describes a impossible range
pub(crate) fn load_span_from<R>(mut source: R, span: Span) -> Result<String>
where
    R: Read,
{
    log::trace!("Loading {:?} from source", &span);
    if span.start.line < 1 {
        bail!("Lines are 1-indexed, can't be less than 1")
    }
    if span.end.line < span.start.line {
        bail!("Line range would be negative, bail")
    }
    if span.end.line == span.start.line && span.end.column < span.start.column {
        bail!("Column range would be negative, bail")
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
    // log::trace!("Loading {:?} from line >{}<", &range, &line);
    Ok(extraction)
}

/// Extract span from a file as `String`.
///
/// Helpful to validate bandaids against what's actually in the file.
#[allow(unused)]
pub(crate) fn load_span_from_file(path: impl AsRef<Path>, span: Span) -> Result<String> {
    let path = path.as_ref();
    let path = path
        .canonicalize()
        .map_err(|e| anyhow!("Failed to canonicalize {}", path.display()).context(e))?;

    let ro = std::fs::OpenOptions::new()
        .read(true)
        .open(&path)
        .map_err(|e| anyhow!("Failed to open {}", path.display()).context(e))?;

    let mut reader = std::io::BufReader::new(ro);

    load_span_from(reader, span)
}

/// Extract a subset of chars by iterating.
/// Range must be in characters.
pub fn sub_chars(s: &str, range: Range) -> String {
    s.chars()
        .skip(range.start)
        .take(range.len())
        .collect::<String>()
}

use core::ops::{Bound, RangeBounds};

/// Extract a subset of chars by iterating.
/// Range must be in characters.
pub fn sub_char_range<'s>(s: &'s str, range: impl RangeBounds<usize>) -> &'s str {
    let mut peekable = s.char_indices().enumerate().peekable();
    let mut byte_range = Range { start: 0, end: 0 };
    let mut started = false;
    'loopy: while let Some((idx, (byte_offset_start, _c))) = dbg!(peekable.next()) {
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
                break 'loopy
            },
            Bound::Excluded(&end) if idx >= end => {
                byte_range.end = byte_offset_start;
                break 'loopy
            },
            _ => {}
        }
        if peekable.peek().is_none() {
            if started {
                byte_range.end = s.len();
            }
        }
    }
    &s[dbg!(byte_range)]
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
        const S: &'static str = r#"
abc
d
"#;
        const S2: &'static str = r#"c
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

        iter_with_line_column(S).zip(EXPECT.into_iter()).for_each(
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
        const SOURCE: &'static str = r##"#[doc=r#"Zebra
Schlupfwespe,
Grünfink"#]"##;
        const S2: &'static str = r#"Zebra
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
        const SOURCE: &'static str = r##"#[doc=r#"Zebra
Schlupfwespe,
"#]"##;
        const S2: &'static str = r#"Zebra
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
        const A: &'static str = "a🐲o🌡i🡴f🕧aodnferntkng";
        const A_EXPECTED: &'static str = "a🐲o";

        assert_eq!(sub_char_range(A, 0..3), A_EXPECTED);
        assert_eq!(sub_char_range(A, ..3), A_EXPECTED);
        assert_eq!(sub_chars(A, 0..3), A_EXPECTED.to_owned());
    }

    #[test]
    fn sub_b() {
        const B: &'static str = "fff🦦🡴🕧";
        const B_EXPECTED: &'static str = "🦦🡴🕧";

        assert_eq!(sub_char_range(B, 3..=5), B_EXPECTED);
        assert_eq!(sub_char_range(B, 3..), B_EXPECTED);
    }

    #[test]
    fn sub_c() {
        const B: &'static str = "fff🦦🡴🕧";
        const B_EXPECTED: &'static str = "";

        assert_eq!(sub_char_range(B, 10..), B_EXPECTED);
        assert_eq!(sub_char_range(B, 15..16), B_EXPECTED);
    }
}
