pub use super::{TrimmedLiteral, TrimmedLiteralDisplay};

use crate::{CheckableChunk, CommentVariant};

use std::fmt;

/// An ordered set of concrete, source-mapped comment literals.
///
/// Documentation literals form one owner's selected documentation variant and may
/// have gaps in source lines. Developer-comment literals are grouped by adjacency.
/// Extraction rejects selections containing unexpanded macros before constructing a set.
/// The literals can be joined into a source-mapped chunk or displayed as text.
#[derive(Clone, Default, Debug, Hash, PartialEq, Eq)]
pub struct LiteralSet {
    /// Concrete literals in document order, each retaining its source span.
    literals: Vec<TrimmedLiteral>,
    /// Inclusive source-line coverage `(start, end)`, which may include unselected gaps.
    pub coverage: (usize, usize),
    /// Syntax of the first literal, used as the set's rendering style.
    variant: CommentVariant,
    /// Attribute syntax or conditional projections prevent whole-block edits.
    non_reflowable: bool,
    cfg_example: Option<String>,
}

impl LiteralSet {
    /// Create a set containing one concrete literal.
    pub fn from(literal: TrimmedLiteral) -> Self {
        Self::from_doc_literals(vec![literal], false)
    }

    /// Build a complete documentation variant from known literals in document order.
    /// Selected literals need not be on adjacent source lines (inactive attributes leave gaps).
    pub(crate) fn from_doc_literals(literals: Vec<TrimmedLiteral>, non_reflowable: bool) -> Self {
        let first = literals
            .first()
            .expect("a documentation variant is nonempty");
        let last = literals.last().unwrap();
        Self {
            coverage: (first.span().start.line, last.span().end.line),
            variant: first.variant(),
            literals,
            non_reflowable,
            cfg_example: None,
        }
    }

    pub(crate) fn set_cfg_example(&mut self, example: String) {
        self.cfg_example = Some(example);
    }

    /// Add a source-adjacent literal from the same comment category.
    ///
    /// Returns the original literal in `Err` if its category or line position does not match.
    /// Documentation cfg selections use `from_doc_literals` instead because source gaps are allowed.
    pub fn add_adjacent(&mut self, literal: TrimmedLiteral) -> Result<(), TrimmedLiteral> {
        if literal.variant().category() != self.variant.category() {
            log::debug!(
                "Adjacent literal is not the same comment variant: {:?} vs {:?}",
                literal.variant().category(),
                self.variant.category()
            );
            return Err(literal);
        }
        let previous_line = literal.span().end.line;
        if previous_line == self.coverage.1 + 1 {
            self.coverage.1 += 1;
            self.literals.push(literal);
            return Ok(());
        }

        let next_line = literal.span().start.line;
        if next_line + 1 == self.coverage.0 {
            self.literals.push(literal);
            self.coverage.1 -= 1;
            return Ok(());
        }

        Err(literal)
    }

    /// Borrow the selected literals in document order.
    pub fn literals(&self) -> Vec<&TrimmedLiteral> {
        self.literals.iter().by_ref().collect()
    }

    /// The number of literals inside this set.
    pub fn len(&self) -> usize {
        self.literals.len()
    }

    /// Whether this set contains no literals.
    pub fn is_empty(&self) -> bool {
        self.literals.is_empty()
    }

    /// Join literal text with synthetic newlines and map each literal back to its source span.
    ///
    /// Source gaps are not copied, and inserted separators have no source mapping.
    pub fn into_chunk(self) -> crate::CheckableChunk {
        let preferred_line_delimiter = self
            .literals
            .iter()
            .find_map(TrimmedLiteral::preferred_line_delimiter);
        let mut source_mapping = indexmap::IndexMap::with_capacity(self.len());
        let mut content = String::with_capacity(self.len() * 120);
        let mut cursor = 0;
        for (index, literal) in self.literals.iter().enumerate() {
            // Inserted separators have no source span. Test the index rather
            // than content.is_empty(): an empty literal still needs a separator.
            if index > 0 {
                content.push('\n');
                cursor += 1;
            }
            let range = cursor..cursor + literal.len_in_chars();
            let span = literal.span();
            // Preserve compatibility with manually constructed legacy macro markers.
            // Documentation extraction rejects unexpanded macros before reaching this layer.
            if literal.variant() != CommentVariant::MacroDocEqMacro {
                if let Some(span_len) = span.one_line_len() {
                    assert_eq!(range.len(), span_len);
                }
            }
            cursor = range.end;
            source_mapping.insert(range, span);
            content.push_str(literal.as_str());
        }
        let mut chunk = CheckableChunk::from_string(content, source_mapping, self.variant);
        // Synthetic separators must not override the selected literals' source style.
        chunk.set_preferred_line_delimiter(preferred_line_delimiter);
        chunk.set_cfg_example(self.cfg_example);
        if self.non_reflowable {
            chunk.disable_reflow();
        }
        chunk
    }
}

impl fmt::Display for LiteralSet {
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
/// A debug helper to print concatenated length of all items.
#[macro_export]
macro_rules! chyrp_dbg {
    ($first:literal $(, $( $line:literal ),+ )? $(,)? $(@ $prefix:literal)? ) => {
        dbg!(concat!($first $( $(, "\n", $line )+ )?).len());
        dbg!(concat!($first $( $(, "\n", $line )+ )?));
    }
}

/// A helper macro creating valid doc string using the macro syntax
/// `#[doc=r#"..."#]`.
///
/// Example:
///
/// ```rust
/// # use doc_chunks::chyrp_up;
/// let x = chyrp_up!(["some", "thing"]);
/// let y = r##"#[doc=r#"some
/// thing"#]
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

/// A helper macro creating valid doc string using the macro syntax
/// `/// ...`.
///
/// Example:
///
/// ```rust
/// # use doc_chunks::fluff_up;
/// let x = fluff_up!(["some", "thing"]);
/// let y = r#"/// some
/// /// thing
/// struct Fluff;"#;
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

pub mod testhelper {
    use super::*;
    use crate::testcase::annotated_literals;

    pub fn gen_literal_set(source: &str) -> LiteralSet {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::testhelper::gen_literal_set;
    use crate::util::load_span_from;
    use crate::util::sub_chars;
    use crate::Range;

    #[test]
    fn set_is_empty() {
        let empty_set = LiteralSet::default();
        assert!(empty_set.is_empty());
        assert_eq!(empty_set.len(), 0);

        let empty_literal =
            TrimmedLiteral::from(CommentVariant::DoubleSlash, "//", 2, 0, 1, 0).unwrap();
        assert!(empty_literal.is_empty());
        let set = LiteralSet::from(empty_literal);
        assert!(!set.is_empty());
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn fluff_one() {
        const RAW: &str = fluff_up!(["a"]);
        const EXPECT: &str = r#"/// a
struct Fluff;"#;
        assert_eq!(RAW, EXPECT);
    }

    #[test]
    fn fluff_multi() {
        const RAW: &str = fluff_up!(["a", "b", "c"]);
        const EXPECT: &str = r#"/// a
/// b
/// c
struct Fluff;"#;
        assert_eq!(RAW, EXPECT);
    }

    #[test]
    fn empty_set_renders_an_empty_chunk() {
        let chunk = LiteralSet::default().into_chunk();
        assert_eq!(chunk.as_str(), "");
        assert_eq!(chunk.variant(), CommentVariant::Unknown);
        assert_eq!(chunk.iter().len(), 0);
        assert!(chunk.is_reflow_safe());
        assert_eq!(chunk.preferred_line_delimiter(), None);
    }

    #[test]
    fn synthetic_separators_do_not_supply_source_newline_metadata() {
        let first =
            TrimmedLiteral::from(CommentVariant::TripleSlash, "/// One", 3, 0, 1, 0).unwrap();
        let second =
            TrimmedLiteral::from(CommentVariant::TripleSlash, "/// Two", 3, 0, 2, 0).unwrap();
        let mut set = LiteralSet::from(first);
        set.add_adjacent(second).unwrap();
        let chunk = set.into_chunk();
        assert_eq!(chunk.as_str(), " One\n Two");
        assert_eq!(chunk.preferred_line_delimiter(), None);
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
            .first()
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
            if range.is_empty() {
                continue;
            }
            assert_eq!(
                load_span_from(RAW.as_bytes(), *span).expect("Span extraction must work"),
                sub_chars(chunk.as_str(), range.clone())
            );

            let r: Range = span.to_content_range(&chunk).expect("Should work");
            // the range for raw str contains an offset of 3 when used with `///`
            assert_eq!(
                sub_chars(chunk.as_str(), range.clone()),
                s.as_str().to_owned()
            );
            assert_eq!(&r, range);
        }
    }
}
