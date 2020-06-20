use crate::markdown::PlainOverlay;
use crate::{LineColumn, Span};

use log::trace;

pub type Range = core::ops::Range<usize>;

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
/// A ref to a trimmed literal.
pub struct TrimmedLiteralRef<'l> {
    reference: &'l TrimmedLiteral,
}

impl<'l> std::ops::Deref for TrimmedLiteralRef<'l> {
    type Target = proc_macro2::Literal;
    fn deref(&self) -> &Self::Target {
        &self.reference.literal
    }
}

impl<'l> From<&'l TrimmedLiteral> for TrimmedLiteralRef<'l> {
    fn from(anno: &'l TrimmedLiteral) -> Self {
        Self { reference: anno }
    }
}

impl<'l> TrimmedLiteralRef<'l> {
    pub fn pre(&self) -> usize {
        self.reference.pre
    }
    pub fn post(&self) -> usize {
        self.reference.pre
    }
    pub fn as_str(&self) -> &str {
        self.reference.as_str()
    }
    pub fn len(&self) -> usize {
        self.reference.len
    }
    pub fn as_ref(&self) -> &TrimmedLiteral {
        self.reference
    }

    #[allow(unused)]
    pub(crate) fn display(&self, highlight: Range) -> TrimmedLiteralDisplay {
        self.reference.display(highlight)
    }
}

impl<'l> fmt::Debug for TrimmedLiteralRef<'l> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.reference.fmt(formatter)
    }
}

/// A literal with meta info where the first and list whitespace may be found.
#[derive(Clone)]
pub struct TrimmedLiteral {
    /// The literal which this annotates to.
    pub literal: proc_macro2::Literal,
    /// the complete rendered string including post and pre.
    pub rendered: String,
    /// Whitespace prefix len + 1
    pub pre: usize,
    /// Whitespace postfix len + 1
    pub post: usize,
    /// Length of rendered **minus** `pre` and `post`.
    /// If the literal is all empty, `pre` and `post` become `0`, and `len` covers the full length of `rendered`.
    pub len: usize,
}

impl std::cmp::PartialEq for TrimmedLiteral {
    fn eq(&self, other: &Self) -> bool {
        if self.rendered != other.rendered {
            return false;
        }
        if self.pre != other.pre {
            return false;
        }
        if self.post != other.post {
            return false;
        }
        if self.len() != other.len() {
            return false;
        }
        if self.literal.span().start() != other.literal.span().start() {
            return false;
        }
        if self.literal.span().end() != other.literal.span().end() {
            return false;
        }

        true
    }
}

impl std::cmp::Eq for TrimmedLiteral {}

impl std::hash::Hash for TrimmedLiteral {
    fn hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        self.rendered.hash(hasher);
        self.pre.hash(hasher);
        self.post.hash(hasher);
        self.len.hash(hasher);
        Span::from(self.literal.span()).hash(hasher);
    }
}

impl From<proc_macro2::Literal> for TrimmedLiteral {
    fn from(literal: proc_macro2::Literal) -> Self {
        let rendered = literal.to_string();
        let raw_start = if rendered.starts_with("r#\"") { 2 } else { 0 };
        let raw_end = if rendered.ends_with("\"#") { 1 } else { 0 };
        let scrap = |c: &'_ char| -> bool { c.is_whitespace() };
        let pre = rendered.chars().take_while(scrap).count() + 1 + raw_start;
        let post = rendered.chars().rev().take_while(scrap).count() + 1 + raw_end;

        let (len, pre, post) = match rendered.len() {
            len if len >= pre + post => (len - pre - post, pre, post),
            len => (len, 0, 0),
        };

        Self {
            len,
            literal,
            rendered,
            pre,
            post,
        }
    }
}

impl std::ops::Deref for TrimmedLiteral {
    type Target = proc_macro2::Literal;
    fn deref(&self) -> &Self::Target {
        &self.literal
    }
}

impl TrimmedLiteral {
    pub fn as_str(&self) -> &str {
        &self.rendered.as_str()[self.pre..(self.pre + self.len)]
    }

    pub fn as_untrimmed_str(&self) -> &str {
        &self.rendered.as_str()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn display(&self, highlight: Range) -> TrimmedLiteralDisplay {
        TrimmedLiteralDisplay::from((self, highlight))
    }
}

impl fmt::Debug for TrimmedLiteral {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use console::Style;

        let pick = Style::new().on_black().underlined().dim().cyan();
        let cutoff = Style::new().on_black().bold().dim().yellow();

        write!(
            formatter,
            "{}{}{}",
            cutoff.apply_to(&self.rendered.as_str()[0..self.pre]),
            pick.apply_to(&self.rendered.as_str()[self.pre..(self.pre + self.len)]),
            cutoff.apply_to(&self.rendered.as_str()[(self.pre + self.len)..]),
        )
    }
}

/// Find the trimmed literal which is covered by the range for `to_string`/`fmt::Display` created str.
///
/// returns a tuple of a literal and Span that is covered by the range
/// but also the `LineColumn` in the proc_macro2 context.
fn find_coverage<'a>(
    literals: &'a [TrimmedLiteral],
    range: &Range,
) -> Option<(Vec<&'a TrimmedLiteral>, LineColumn, LineColumn)> {
    let core::ops::Range::<usize> { start, end } = range;
    let mut offset = *start;
    let length = end - start;
    assert!(length > 0);

    #[derive(Copy, Clone, Debug)]
    enum LookingFor {
        Start,
        End { start: LineColumn },
    }

    let mut acc = Vec::with_capacity(8);
    let mut state = LookingFor::Start;
    let mut it = literals.iter();
    let mut opt = it.next();
    loop {
        opt = if let Some(literal) = opt {
            // work on the string version length
            // such that we have the paddings removed
            // since this is what is sent to the checker

            // the string repr is a concatentation of all trimmed strings
            // so we have to account for that with the line length
            let len = literal.as_str().len() + 1; // account for the introduced newline

            assert_eq!(literal.span().start().line, literal.span().end().line);
            state = match state {
                LookingFor::Start => {
                    if offset >= len {
                        offset -= len;
                        // offset += 1; // additional \n introduced when combining literals
                        LookingFor::Start
                    } else {
                        trace!(
                            "Start = {} + {} + {}",
                            literal.span().start().column,
                            offset,
                            literal.pre
                        );
                        state = LookingFor::End {
                            start: LineColumn {
                                line: literal.span().start().line,
                                // add the padding again, to make for a sane global span
                                column: literal.span().start().column + offset + literal.pre,
                            },
                        };
                        // the new offset we are looking for
                        offset += length;
                        // do not advance the iterator, we need to check the same line for the end too!
                        continue;
                    }
                }

                LookingFor::End { start } => {
                    acc.push(literal);
                    if offset >= len {
                        offset -= len;
                        // offset += 1; // additional \n introduced when combining literals
                        state
                    } else {
                        trace!(
                            "End = {} + {} + {} - 1",
                            literal.span().start().column,
                            offset,
                            literal.pre
                        );
                        let end = LineColumn {
                            // @todo assumes start and end are on the same line for the literal
                            line: literal.span().start().line,
                            // add the padding again, to make for a sane global span
                            // substract -1 since line column are inclusive and offset += length yields exclusive
                            column: literal.span().start().column + offset + literal.pre - 1,
                        };
                        assert_eq!(start.line, end.line);
                        // if start and end column are equiv, this is a one character match
                        return Some((acc, start, end));
                    }
                }
            };
            it.next()
        } else {
            break;
        };
    }
    None
}

/// A set of consecutive literals.
///
/// Provides means to render them as a code block
#[derive(Clone, Default, Debug, Hash, PartialEq, Eq)]
pub struct LiteralSet {
    /// consecutive set of literals mapped by line number
    literals: Vec<TrimmedLiteral>,
    /// lines spanned (start, end)
    pub coverage: (usize, usize),
}

impl LiteralSet {
    /// Initiate a new set based on the first literal
    pub fn from(literal: TrimmedLiteral) -> Self {
        Self {
            coverage: (literal.span().start().line, literal.span().end().line),
            literals: vec![literal],
        }
    }

    /// Create a plain overlay to work on.
    pub fn erase_markdown(&self) -> PlainOverlay {
        PlainOverlay::erase_markdown(self)
    }

    /// Overwrite the actual literal content with fixed content.
    ///
    /// Commonly this means with suggestions applied, content can
    /// contain newlines.
    pub fn replace_content(&mut self, _content: String) {
        unimplemented!("")
    }

    /// Add a literal to a literal set, if the previous lines literal already exists.
    ///
    /// Returns literl within the Err variant if not adjacent
    pub fn add_adjacent(&mut self, literal: TrimmedLiteral) -> Result<(), TrimmedLiteral> {
        let previous_line = literal.span().end().line;
        if previous_line == self.coverage.1 + 1 {
            self.coverage.1 += 1;
            let _ = self.literals.push(literal);
            return Ok(());
        }

        let next_line = literal.span().start().line;
        if next_line + 1 == self.coverage.0 {
            let _ = self.literals.push(literal);
            self.coverage.1 -= 1;
            return Ok(());
        }

        Err(literal)
    }

    /// Convert a range of the linear trimmed (but no other processing) string representation to a set of
    /// literal references and spans within that literal (spans on the proc_macro2 literal).
    pub fn linear_range_to_spans<'a>(
        &'a self,
        range: core::ops::Range<usize>,
    ) -> Vec<(&'a TrimmedLiteral, Span)> {
        find_coverage(&self.literals, &range)
            .map(|(literals, start, end)| {
                assert!(!literals.is_empty());
                trace!("coverage: {:?} -> end {:?}", &range, end);
                let n = literals.len();
                if n > 1 {
                    let mut iter = literals.into_iter();
                    let first: &'a _ = iter.next().unwrap();

                    // calculate how many lines it spans
                    let mut acc = Vec::with_capacity(n);
                    // first literal to its end
                    if first.span().end() != start {
                        acc.push((
                            first,
                            Span {
                                start,
                                end: first.span().end(),
                            },
                        ));
                    }

                    // take all in between the first and the last completely

                    for literal in iter.clone().take(n - 2) {
                        let span = Span {
                            start: literal.span().start(),
                            end: literal.span().end(),
                        };
                        if span.start != span.end {
                            acc.push((literal, span));
                        }
                    }
                    // add the last from the beginning to the computed end
                    let last: &'a _ = iter.skip(n - 2).next().unwrap();
                    if last.span().start() != end {
                        acc.push((
                            last,
                            Span {
                                start: last.span().start(),
                                end,
                            },
                        ));
                    }
                    return acc;
                } else {
                    assert_eq!(n, 1);
                    return vec![(literals.first().unwrap(), Span { start, end })];
                }
            })
            .unwrap_or_else(|| Vec::new())
    }

    pub fn literals<'x>(&'x self) -> Vec<&'x TrimmedLiteral> {
        self.literals.iter().by_ref().collect()
    }

    pub fn len(&self) -> usize {
        self.literals.len()
    }

    // pub fn path(&self) -> Option<PathBuf> {
    //     self.literals.iter().next().map(|x| x.span().source_file().path())
    // }
}

use std::fmt;

impl<'s> fmt::Display for LiteralSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.len();
        if n > 0 {
            for literal in self.literals.iter().take(n - 1) {
                writeln!(formatter, "{}", literal.as_str())?;
            }
            if let Some(literal) = self.literals.last() {
                write!(formatter, "{}", literal.as_str())?;
            }
        }
        Ok(())
    }
}

/// A display style wrapper for a trimmed literal.
///
/// Allows better display of coverage results without code duplication.
///
/// Consists of literal reference and a relative range to the start of the literal.
#[derive(Debug, Clone)]
pub(crate) struct TrimmedLiteralDisplay<'a>(pub TrimmedLiteralRef<'a>, pub Range);

impl<'a, R> From<(R, Range)> for TrimmedLiteralDisplay<'a>
where
    R: Into<TrimmedLiteralRef<'a>>,
{
    fn from(tuple: (R, Range)) -> Self {
        let tuple0 = tuple.0.into();
        Self(tuple0, tuple.1)
    }
}

impl<'a> Into<(TrimmedLiteralRef<'a>, Range)> for TrimmedLiteralDisplay<'a> {
    fn into(self) -> (TrimmedLiteralRef<'a>, Range) {
        (self.0, self.1)
    }
}

impl<'a> fmt::Display for TrimmedLiteralDisplay<'a> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use console::Style;

        // part that is hidden by the trimmed literal, but still present in the actual literal
        let cutoff = Style::new().on_black().bold().underlined().yellow();
        // the contextual characters not covered by range `self.1`
        let context = Style::new().on_black().bold().cyan();
        // highlight the mistake
        let highlight = Style::new().on_black().bold().underlined().red().italic();
        // a special style for any errors, to visualize out of bounds access
        let oob = Style::new().blink().bold().on_yellow().red();

        // simplify
        let literal = self.0;
        let start = self.1.start;
        let end = self.1.end;

        assert!(start <= end);

        // content without quote characters
        let data = literal.as_ref().rendered.as_str();

        // colour the preceding quote character
        // and the context preceding the highlight
        let (pre, ctx1) = if start > literal.pre() {
            (
                cutoff.apply_to(&data[..literal.pre()]).to_string(),
                context.apply_to(&data[literal.pre()..start]).to_string(),
            )
        } else if start <= data.len() {
            (cutoff.apply_to(&data[..start]).to_string(), String::new())
        } else {
            (String::new(), "!!!".to_owned())
        };
        // highlight the given range
        let highlight = if end >= data.len() {
            oob.apply_to(&data[start..data.len()]).to_string()
        } else {
            highlight.apply_to(&data[start..end]).to_string()
        };
        // color trailing context if any as well as the closing quote character
        let post_idx = literal.pre() + literal.len();
        let (ctx2, post) = if post_idx > end {
            (
                context.apply_to(&data[end..post_idx]).to_string(),
                cutoff.apply_to(&data[post_idx..]).to_string(),
            )
        } else if end < data.len() {
            (String::new(), cutoff.apply_to(&data[end..]).to_string())
        } else {
            (String::new(), oob.apply_to("!!!").to_string())
        };

        write!(formatter, "{}{}{}{}{}", pre, ctx1, highlight, ctx2, post)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const SKIP: usize = 3;

    const EXMALIBU_RANGE_START: usize = SKIP + 9;
    const EXMALIBU_RANGE_END: usize = EXMALIBU_RANGE_START + 8;
    const EXMALIBU_RANGE: Range = EXMALIBU_RANGE_START..EXMALIBU_RANGE_END;
    const TEST: &str = r#"/// Another exmalibu verification pass.
///
/// Boats float, don't they?
struct Vikings;
"#;

    const TEST_LITERALS_COMBINED: &str = r#" Another exmalibu verification pass.

 Boats float, don't they?"#;

    pub(crate) fn annotated_literals(source: &str) -> Vec<TrimmedLiteral> {
        let stream =
            syn::parse_str::<proc_macro2::TokenStream>(source).expect("Must be valid rust");
        stream
            .into_iter()
            .filter_map(|x| {
                if let proc_macro2::TokenTree::Group(group) = x {
                    Some(group.stream().into_iter())
                } else {
                    None
                }
            })
            .flatten()
            .filter_map(|x| {
                if let proc_macro2::TokenTree::Literal(literal) = x {
                    Some(literal)
                } else {
                    None
                }
            })
            .map(|literal| TrimmedLiteral::from(literal))
            .collect()
    }

    pub(crate) fn gen_literal_set(_source: &str) -> LiteralSet {
        let literals = dbg!(annotated_literals(TEST));

        let mut cls = LiteralSet::default();
        for literal in literals {
            assert!(dbg!(&mut cls).add_adjacent(literal).is_ok());
        }
        cls
    }

    #[test]
    fn combine_literals() {
        let _ = env_logger::builder().is_test(true).try_init();

        let cls = gen_literal_set(TEST);

        assert_eq!(cls.len(), 3);
        assert_eq!(cls.to_string(), TEST_LITERALS_COMBINED.to_string());
    }

    #[test]
    fn coverage() {
        let _ = env_logger::builder().is_test(true).try_init();

        let trimmed_literals = annotated_literals(TEST);
        let (literals, _start, _end) =
            dbg!(find_coverage(trimmed_literals.as_slice(), &EXMALIBU_RANGE)).unwrap();
        let literal = literals.first().expect("Must be at least one literal");

        let range_for_raw_test_str = Range {
            start: EXMALIBU_RANGE_START - SKIP,
            end: EXMALIBU_RANGE_END - SKIP,
        };
        assert_eq!(
            "exmalibu",
            &literal.as_str()[range_for_raw_test_str.clone()]
        );
        assert_eq!(
            &TEST[EXMALIBU_RANGE],
            &literal.as_str()[range_for_raw_test_str]
        );
    }

    macro_rules! test_raw {
        ($test: ident $(, $literal: literal)+ ; $range: expr, $expected: literal) => {
            #[test]
            fn $test() {
                let _ = env_logger::builder().filter(None, log::LevelFilter::Trace).is_test(true).try_init();

                const TEST: &str = concat!("" $(, "///", $literal, "\n")+ , "struct X;");
                const START: usize = 3;
                let _end: usize = START + vec![$($literal.len()),* ].into_iter().sum::<usize>();
                let literals = annotated_literals(TEST);

                let range: Range = $range;

                let (literals, _start, _end) =
                find_coverage(literals.as_slice(), &range).expect("Must find coverage");
                let literal = literals.first().expect("Must be at least one literal");
                let range_for_raw_str = Range {
                    start: range.start + START,
                    end: range.end + START,
                };

                assert_eq!(&TEST[range_for_raw_str.clone()], &literal.as_str()[$range]);
                assert_eq!(&TEST[range_for_raw_str], $expected);
            }
        };
    }

    test_raw!(raw_extract_0, " livelyness", " yyy" ; 2..6, "ivel");
    test_raw!(raw_extract_1, " + 12 + x0" ; 9..10, "0");
}
