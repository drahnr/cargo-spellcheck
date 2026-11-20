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
        start: LineColumn { line: 1, column: 9 }, // prefix is #[doc=r#"
        end: LineColumn { line: 3, column: 7 },   // suffix is pointeless
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
        start: LineColumn { line: 1, column: 9 }, // prefix is #[doc=r#"
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
