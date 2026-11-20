use super::*;

use super::testhelper::gen_literal_set;
use crate::fluff_up;
use crate::util::load_span_from;
use crate::util::sub_chars;

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
    let set = LiteralSet::default();
    let expected_content = "";
    let expected_variant = CommentVariant::Unknown;
    let expected_fragment_count = 0;
    let expected_delimiter = None;
    let chunk = set.into_chunk();
    assert_eq!(chunk.as_str(), expected_content);
    assert_eq!(chunk.variant(), expected_variant);
    assert_eq!(chunk.iter().len(), expected_fragment_count);
    assert!(chunk.is_reflow_safe());
    assert_eq!(chunk.preferred_line_delimiter(), expected_delimiter);
}

#[test]
fn synthetic_separators_do_not_supply_source_newline_metadata() {
    let first = TrimmedLiteral::from(CommentVariant::TripleSlash, "/// One", 3, 0, 1, 0).unwrap();
    let second = TrimmedLiteral::from(CommentVariant::TripleSlash, "/// Two", 3, 0, 2, 0).unwrap();
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
