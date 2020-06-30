use crate::{Range, LineColumn, Span, CheckableChunk};
pub use super::{TrimmedLiteral,TrimmedLiteralRef, TrimmedLiteralDisplay};
/// A set of consecutive literals.
///
/// Provides means to render them as a code block
#[derive(Clone, Default, Debug, Hash, PartialEq, Eq)]
pub struct LiteralSet {
    /// consecutive set of literals mapped by line number
    literals: Vec<TrimmedLiteral>,
    /// lines spanned (start, end) inclusive
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

    pub fn literals<'x>(&'x self) -> Vec<&'x TrimmedLiteral> {
        self.literals.iter().by_ref().collect()
    }

    pub fn len(&self) -> usize {
        self.literals.len()
    }

    pub fn into_chunk(self) -> crate::documentation::CheckableChunk {
        let n = self.len();
        let mut source_mapping = indexmap::IndexMap::with_capacity(n);
        let mut content = String::with_capacity(n * 120);
        if n > 0 {
            let mut cursor = 0usize;
            // for use with `Range`
            let mut start; // inclusive
            let mut end ; // exclusive
            for literal in self.literals.iter().take(n - 1) {
                start = cursor;
                cursor += literal.len();
                end = cursor;
                // @todo check if the `Span` conversion here is done correctly
                let mut span = Span::from(literal);
                source_mapping.insert(Range { start, end },
                    span
                );
                content.push_str(literal.as_str());
                content.push('\n');
                // the newline is _not_ covered by a span, after all it's inserted by us!
                cursor += 1;
            }
            if let Some(literal) = self.literals.last() {
                start = cursor;
                cursor += literal.len();
                end = cursor;
                let span = Span::from(literal);
                source_mapping.insert(Range { start, end }, span);
                content.push_str(literal.as_str());
                // for the last, skip the newline
            }
        }
        CheckableChunk::from_string(content, source_mapping)
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
    use super::*;
    pub(crate) use super::super::literal::tests::annotated_literals;

    #[macro_export]
    macro_rules! fluff_up {
        ([ $( $line:literal ),+ $(,)?]) => {
            concat!("/// ", fluff_up!( $( $line ),+ ), "\nstruct Fluff;");
        };
        ($acc:literal, $( $line:literal ),+ $(,)?) => {
            concat!($acc, "\n/// ", fluff_up!( $( $line ),+ ))
        };
        ($leaf:literal) => {
            concat!($leaf)
        };
    }

    #[test]
    fn fluff_one() {
        const TEST: &'static str = fluff_up!(["a"]);
        const EXPECT: &'static str = r#"/// a
struct Fluff;"#;
        assert_eq!(TEST, EXPECT);
    }

    #[test]
    fn fluff_multi() {
        const TEST: &'static str = fluff_up!(["a","b","c"]);
        const EXPECT: &'static str = r#"/// a
/// b
/// c
struct Fluff;"#;
        assert_eq!(TEST, EXPECT);
    }

    /// prefer `fluff_up!` over this
    #[allow(unused)]
    pub(crate) fn gen_literal_set_with_fluff(source: &str) -> LiteralSet {
        let mut fluffed = String::with_capacity(source.len() + 32);
        for line in source.lines() {
            fluffed.push_str("/// ");
            fluffed.push_str(line);
            fluffed.push('\n');
        }
        fluffed.push_str("struct X{}");
        gen_literal_set(fluffed.as_str())
    }


    pub(crate) fn gen_literal_set(source: &str) -> LiteralSet {
        let literals = dbg!(annotated_literals(dbg!(source)));

        let mut cls = LiteralSet::default();
        for literal in literals {
            assert!(cls.add_adjacent(literal).is_ok());
        }
        dbg!(cls)
    }


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

        let literal_set = gen_literal_set(TEST);
        let chunk: CheckableChunk = literal_set.into_chunk();
        let map_range_to_span = chunk.find_spans(EXMALIBU_RANGE);
        let (range, span) = map_range_to_span
            .iter()
            .next()
            .expect("Must be at least one literal");

        let range_for_raw_str = Range {
            start: EXMALIBU_RANGE_START - SKIP,
            end: EXMALIBU_RANGE_END - SKIP,
        };

        // check test integrity
        assert_eq!("exmalibu", &TEST[EXMALIBU_RANGE]);

        // check actual result
        assert_eq!(
            &TEST[EXMALIBU_RANGE],
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

            const TEST: &str = concat!("" $(, "///", $txt, "\n")+ , "struct X;");
            const START: usize = 3; // skip `///` which the span we get from the literal
            let _end: usize = START + vec![$($txt.len()),* ].into_iter().sum::<usize>();
            let literal_set = gen_literal_set(TEST);


            let chunk: CheckableChunk = literal_set.into_chunk();
            let map_range_to_span = chunk.find_spans(range.clone());
            let mut iter = dbg!(map_range_to_span).into_iter();
            let (range, _span) = iter.next().expect("Must be at least one literal");
            let range_for_raw_str = Range {
                start: range.start + START,
                end: range.end + START,
            };

            // @todo check test data integrity here
            assert_eq!(&TEST[range_for_raw_str.clone()], &chunk.as_str()[range_for_raw_str.clone()]);
            assert_eq!(&TEST[range_for_raw_str], $expected);

        };
    }

    // @todo tests used to be good, so the `find_spans` implementation must still be flawed :)
    test_raw!(raw_extract_0, [" livelyness", " yyy"] ; 2..6, "ivel");
    test_raw!(raw_extract_1, [" + 12 + x0"] ; 9..10, "0");
}
