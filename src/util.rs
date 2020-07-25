use anyhow::{anyhow,bail,Error,Result};
use std::io::Read;
use crate::{LineColumn, Span};


/// Iterate over a str and annotate with line and column.
///
/// Assumes `s` is content starting from point `start_point`.
pub fn iter_with_line_column_from<'a>(s: &'a str, start_point: LineColumn) -> impl Iterator<Item=(char, usize, LineColumn)> + 'a
{
	#[derive(Clone)]
	struct State {
		cursor: LineColumn,
		previous_char_was_newline: bool,
	};
	let mut initial = State{
		cursor: LineColumn {line: 1, column: 0},
		previous_char_was_newline: false,
	};

	s.chars().enumerate().scan(initial, |state, (idx, c)| -> Option<_> {
		let cursor = state.cursor;
		state.previous_char_was_newline = c == '\n';
		if state.previous_char_was_newline {
			state.cursor.line += 1;
			state.cursor.column = 0;
		} else {
			state.cursor.column += 1;
		}
		Some((c, idx, cursor))
	})
}

/// Iterate over annotated chars starting from line 1 and column 0 assuming `s` starts there.
pub fn iter_with_line_column<'a>(s: &'a str) -> impl Iterator<Item=(char, usize, LineColumn)> + 'a {
	iter_with_line_column_from(s, LineColumn { line: 1, column: 0})
}

/// Extract `span` from a `Read`-able source as `String`
pub fn load_span_from<R>(mut source: R, span: Span) -> Result<String>
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
	let cursor = (LineColumn { line: 0, column: 0 }, true);
	let extraction = iter_with_line_column(s.as_str())
		.filter_map(|(c, idx, cursor)| {
			if cursor.line < span.start.line {
				return None;
			}
			if cursor.line > span.end.line {
				return None;
			}
			// bounding lines
			if cursor.line == span.start.line && cursor.column < span.start.column {
				return None;
			}
			if cursor.line == span.end.line && cursor.column > span.end.column {
				return None;
			}
			Some(c)
		})
		.collect::<String>();
	// log::trace!("Loading {:?} from line >{}<", &range, &line);
	Ok(extraction)
}



#[cfg(test)]
mod tests {
	use super::*;
	macro_rules! lcc {
		($line:literal, $column:literal, $c:literal) => {
			(LineColumn {
				line: $line,
				column: $column,
			}, $c)
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
			lcc!(1,0,'\n'),
			lcc!(2,0,'a'),
			lcc!(2,1,'b'),
			lcc!(2,2,'c'),
			lcc!(2,3,'\n'),
			lcc!(3,0,'d'),
			lcc!(3,1,'\n'),
		];

		iter_with_line_column(S).zip(EXPECT.into_iter()).for_each(|((c, _idx, lc), (expected_lc, expected_c))| {
			assert_eq!(lc, expected_lc.clone());
			assert_eq!(c, expected_c.clone());
		});



		const SPAN: Span = Span {
			start: LineColumn { line: 2, column: 2 },
			end: LineColumn { line: 3, column: 0,},
		};

		assert_eq!(load_span_from(&mut S.as_bytes(), SPAN).expect("Must succeed"), S2.to_owned());

	}


	#[test]
	fn iter_span_doc() {
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

		assert_eq!(load_span_from(&mut SOURCE.as_bytes(), SPAN).expect("Must succeed"), S2.to_owned());

	}
}