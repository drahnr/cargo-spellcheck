pub use super::{TrimmedLiteral, TrimmedLiteralDisplay};
use crate::{CheckableChunk, CommentVariant, Range};
/// A set of consecutive literals.
///
/// Provides means to render them as a code block
#[derive(Clone, Default, Debug, Hash, PartialEq, Eq)]
pub struct LiteralSet {
    /// consecutive set of literals mapped by line number
    literals: Vec<TrimmedLiteral>,
    /// lines spanned (start, end) inclusive
    pub coverage: (usize, usize),
    /// Track what kind of comment the literals are
    variant: CommentVariant,
}

impl LiteralSet {
    /// Initiate a new set based on the first literal
    pub fn from(literal: TrimmedLiteral) -> Self {
        Self {
            coverage: (literal.span().start.line, literal.span().end.line),
            variant: literal.variant(),
            literals: vec![literal],
        }
    }

    /// Add a literal to a literal set, if the previous lines literal already exists.
    ///
    /// Returns literl within the Err variant if not adjacent
    pub fn add_adjacent(&mut self, literal: TrimmedLiteral) -> Result<(), TrimmedLiteral> {
        if literal.variant() != self.variant {
            log::error!(
                "Adjacent literal is not the same comment variant: {:?} vs {:?}",
                literal.variant(),
                self.variant
            );
            return Err(literal);
        }
        let previous_line = literal.span().end.line;
        if previous_line == self.coverage.1 + 1 {
            self.coverage.1 += 1;
            let _ = self.literals.push(literal);
            return Ok(());
        }

        let next_line = literal.span().start.line;
        if next_line + 1 == self.coverage.0 {
            let _ = self.literals.push(literal);
            self.coverage.1 -= 1;
            return Ok(());
        }

        Err(literal)
    }

    /// The set of trimmed literals that is covered.
    pub fn literals<'x>(&'x self) -> Vec<&'x TrimmedLiteral> {
        self.literals.iter().by_ref().collect()
    }

    /// The number of literals inside this set.
    pub fn len(&self) -> usize {
        self.literals.len()
    }

    /// Convert to a checkable chunk.
    ///
    /// Creates the map from content ranges to source spans.
    pub fn into_chunk(self) -> crate::documentation::CheckableChunk {
        let n = self.len();
        let mut source_mapping = indexmap::IndexMap::with_capacity(n);
        let mut content = String::with_capacity(n * 120);
        if n > 0 {
            // cursor operates on characters
            let mut cursor = 0usize;
            // for use with `Range`
            let mut start; // inclusive
            let mut end; // exclusive
            let mut it = self.literals.iter();
            let mut next = it.next();
            while let Some(literal) = next {
                start = cursor;
                cursor += literal.len_in_chars();
                end = cursor;

                let span = literal.span();
                let range = Range { start, end };

                if let Some(span_len) = span.one_line_len() {
                    assert_eq!(range.len(), span_len);
                }
                // keep zero length values too, to guarantee continuity
                source_mapping.insert(range, span);
                content.push_str(literal.as_str());
                // the newline is _not_ covered by a span, after all it's inserted by us!
                next = it.next();
                if next.is_some() {
                    // for the last, skip the newline
                    content.push('\n');
                    cursor += 1;
                }
            }
        }
        // all literals in a set have the same variant, so lets take the first one
        let variant = if let Some(literal) = self.literals.first() {
            literal.variant()
        } else {
            crate::CommentVariant::Unknown
        };
        CheckableChunk::from_string(content, source_mapping, variant)
    }
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

#[cfg(test)]
pub(crate) mod tests {
    pub(crate) use super::super::literal::tests::annotated_literals;
    use super::*;

    /// A debug helper to print concatenated length of all items.
    #[macro_export]
    macro_rules! chyrp_dbg {
        ($first:literal $(, $( $line:literal ),+ )? $(,)? $(@ $prefix:literal)? ) => {
            dbg!(concat!($first $( $(, "\n", $line )+ )?).len());
            dbg!(concat!($first $( $(, "\n", $line )+ )?));
        }
    }

    /// A helper macro creating valid doc string using
    /// the macro syntax `#[doc=r#"..."#]`.
    ///
    /// Example:
    ///
    /// ```rust
    /// let x = chryp_up!(["some", "thing"])
    /// let y = r##"#[doc=r#"some
    /// thing"#
    /// struct ChyrpChyrp;"##;
    ///
    /// assert_eq!(x,y);
    /// ```
    #[macro_export]
    macro_rules! chyrp_up {
        ([ $( $line:literal ),+ $(,)? ] $(@ $prefix:literal)? ) => {
            chyrp_up!( $( $line ),+ $(@ $prefix)? )
        };
        ($first:literal $(, $( $line:literal ),+ )? $(,)? $(@ $prefix:literal)? ) => {
            concat!($( $prefix ,)? r##"#[doc=r#""##, $first $( $(, "\n", $line )+ )?, r##""#]"##, "\n", "struct ChyrpChyrp;")
        };
    }

    /// A helper macro creating valid doc string using
    /// the macro syntax `/// ...`.
    ///
    /// Example:
    ///
    /// ```rust
    /// let x = fluff_up!(["some", "thing"])
    /// let y = r#"/// some
    /// /// thing
    /// struct Fluff;"##;
    ///
    /// assert_eq!(x,y);
    /// ```
    #[macro_export]
    macro_rules! fluff_up {
        ([ $( $line:literal ),+ $(,)?] $( @ $prefix:literal)?) => {
            fluff_up!($( $line ),+ $(@ $prefix)?)
        };
        ($($line:literal ),+ $(,)? ) => {
            fluff_up!($( $line ),+ @ "")
        };
        ($($line:literal ),+ $(,)? @ $prefix:literal ) => {
            concat!("" $(, $prefix, "/// ", $line, "\n")+ , "struct Fluff;")
        };
    }

    #[test]
    fn fluff_one() {
        const RAW: &'static str = fluff_up!(["a"]);
        const EXPECT: &'static str = r#"/// a
struct Fluff;"#;
        assert_eq!(RAW, EXPECT);
    }

    #[test]
    fn fluff_multi() {
        const RAW: &'static str = fluff_up!(["a", "b", "c"]);
        const EXPECT: &'static str = r#"/// a
/// b
/// c
struct Fluff;"#;
        assert_eq!(RAW, EXPECT);
    }

    pub(crate) fn gen_literal_set(source: &str) -> LiteralSet {
        let literals = dbg!(annotated_literals(dbg!(source)));

        let mut iter = dbg!(literals).into_iter();
        let literal = iter
            .next()
            .expect("Must have at least one item in laterals");
        let mut cls = LiteralSet::from(literal);

        for literal in iter {
            assert!(cls.add_adjacent(literal).is_ok());
        }
        dbg!(cls)
    }

    // range within the literalset content string
    const EXMALIBU_RANGE_START: usize = 9;
    const EXMALIBU_RANGE_END: usize = EXMALIBU_RANGE_START + 8;
    const EXMALIBU_RANGE: Range = EXMALIBU_RANGE_START..EXMALIBU_RANGE_END;
    const RAW: &str = r#"/// Another exmalibu verification pass.
/// 🚤w🌴x🌋y🍈z🍉0
/// ♫ Boats float, ♫♫ don't they? ♫
struct Vikings;
"#;

    const EXMALIBU_CHUNK_STR: &str = r#" Another exmalibu verification pass.
 🚤w🌴x🌋y🍈z🍉0
 ♫ Boats float, ♫♫ don't they? ♫"#;

    #[test]
    fn combine_literals() {
        let _ = env_logger::builder()
            .is_test(true)
            .filter(None, log::LevelFilter::Trace)
            .try_init();

        let cls = gen_literal_set(RAW);

        assert_eq!(cls.len(), 3);
        assert_eq!(cls.to_string(), EXMALIBU_CHUNK_STR.to_owned());
    }

    #[test]
    fn coverage() {
        let _ = env_logger::builder()
            .is_test(true)
            .filter(None, log::LevelFilter::Trace)
            .try_init();

        let literal_set = gen_literal_set(RAW);
        let chunk: CheckableChunk = literal_set.into_chunk();
        let map_range_to_span = chunk.find_spans(EXMALIBU_RANGE);
        let (_range, _span) = map_range_to_span
            .iter()
            .next()
            .expect("Must be at least one literal");

        let range_for_raw_str = Range {
            start: EXMALIBU_RANGE_START,
            end: EXMALIBU_RANGE_END,
        };

        // check test integrity
        assert_eq!("exmalibu", &EXMALIBU_CHUNK_STR[EXMALIBU_RANGE]);

        // check actual result
        assert_eq!(
            &EXMALIBU_CHUNK_STR[EXMALIBU_RANGE],
            &chunk.as_str()[range_for_raw_str.clone()]
        );
    }

    macro_rules! test_raw {
        ($test: ident, [ $($txt: literal),+ $(,)? ]; $range: expr, $expected: literal) => {
            #[test]
            fn $test() {
                test_raw!([$($txt),+] ; $range, $expected);
            }
        };

        ([$($txt:literal),+ $(,)?]; $range: expr, $expected: literal) => {
            let _ = env_logger::builder()
                .filter(None, log::LevelFilter::Trace)
                .is_test(true)
                .try_init();

            let range: Range = $range;

            const RAW: &str = fluff_up!($( $txt),+);
            const START: usize = 3; // skip `///` which is the span we get from the literal
            let _end: usize = START $( + $txt.len())+;
            let literal_set = gen_literal_set(dbg!(RAW));


            let chunk: CheckableChunk = dbg!(literal_set.into_chunk());
            let map_range_to_span = chunk.find_spans(range.clone());

            let mut iter = dbg!(map_range_to_span).into_iter();
            let (range, _span) = iter.next().expect("Must be at least one literal");

            // the range for raw str contains an offset of 3 when used with `///`
            let range_for_raw_str = Range {
                start: range.start + START,
                end: range.end + START,
            };

            assert_eq!(&RAW[range_for_raw_str.clone()], &chunk.as_str()[range], "Testing range extract vs stringified chunk for integrity");
            assert_eq!(&RAW[range_for_raw_str], $expected, "Testing range extract vs expected");
        };
    }

    #[test]
    fn first_line_extract_0() {
        test_raw!(["livelyness", "yyy"] ; 2..6, "ivel");
    }

    #[test]
    fn first_line_extract_1() {
        test_raw!(["+ 12 + x0"] ; 9..10, "0");
    }

    use crate::util::load_span_from;

    #[test]
    fn literal_set_into_chunk() {
        let _ = env_logger::builder()
            .filter(None, log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        let literal_set = dbg!(gen_literal_set(RAW));

        let chunk = dbg!(literal_set.clone().into_chunk());
        let it = literal_set.literals();

        for (range, span, s) in itertools::cons_tuples(chunk.iter().zip(it)) {
            if range.len() == 0 {
                continue;
            }
            assert_eq!(
                load_span_from(RAW.as_bytes(), span.clone()).expect("Span extraction must work"),
                crate::util::sub_chars(chunk.as_str(), range.clone())
            );

            let r: Range = span.to_content_range(&chunk).expect("Should work");
            // the range for raw str contains an offset of 3 when used with `///`
            assert_eq!(
                crate::util::sub_chars(chunk.as_str(), range.clone()),
                s.as_str().to_owned()
            );
            assert_eq!(&r, range);
        }
    }
}
