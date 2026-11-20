use crate::sub_chars;

use super::*;

#[test]
fn tokenize_for_abbrev_sentence() {
    let tok = tokenizer::<PathBuf>(None).unwrap();
    let ranges = apply_tokenizer(&tok, "It isn't that different.");

    ranges
        .zip([0_usize..2, 3..8, 9..13, 14..23].iter().cloned())
        .for_each(|(is, expect)| {
            assert_eq!(is, expect);
        });
}

#[test]
fn tokenize_for_abbrev_short() {
    let tok = tokenizer::<PathBuf>(None).unwrap();
    let mut ranges = apply_tokenizer(&tok, "isn't");
    assert_eq!(ranges.next(), Some(0_usize..5));
}

#[test]
fn tokenize_ink_bang_0_tick_s() {
    let tok = tokenizer::<PathBuf>(None).unwrap();
    let mut ranges = apply_tokenizer(&tok, "ink!'s");

    assert_eq!(ranges.next(), Some(0_usize..6));
}

#[test]
fn tokenize_ink_bang_1_tick_s_w_brackets() {
    let tok = tokenizer::<PathBuf>(None).unwrap();
    let mut ranges = apply_tokenizer(&tok, "(ink!'s)");

    assert_eq!(ranges.next(), Some(0_usize..1));
    assert_eq!(ranges.next(), Some(1_usize..7));
    assert_eq!(ranges.next(), Some(7_usize..8));
}

#[test]
fn tokenize_ink_bang_2_tick_s_w_brackets_spaced() {
    let tok = tokenizer::<PathBuf>(None).unwrap();
    let mut ranges = apply_tokenizer(&tok, "( ink!'s )");

    assert_eq!(ranges.next(), Some(0_usize..1));
    assert_eq!(ranges.next(), Some(2_usize..8));
    assert_eq!(ranges.next(), Some(9_usize..10));
}

#[test]
fn tokenize_single_ticks_w_brackets() {
    let tok = tokenizer::<PathBuf>(None).unwrap();
    let ranges = apply_tokenizer(&tok, "the ('lock funds') transaction");

    ranges
        .zip([0_usize..3, 4..5, 5..6, 6..10, 11..16].iter().cloned())
        .for_each(|(is, expect)| {
            assert_eq!(is, expect);
        });
}

#[test]
fn tokenize_double_ticks() {
    let tok = tokenizer::<PathBuf>(None).unwrap();
    let ranges = apply_tokenizer(&tok, r#"the "lock funds" transaction"#);

    ranges
        .zip(
            [0_usize..3, 4..5, 5..9, 10..15, 15..16, 17..28]
                .iter()
                .cloned(),
        )
        .for_each(|(is, expect)| {
            assert_eq!(is, expect);
        });
}

#[test]
fn tokenize_bracketed_w_tick_s_inside() {
    let tok = tokenizer::<PathBuf>(None).unwrap();
    let ranges = apply_tokenizer(&tok, r#"the (Xyz's) do"#);

    ranges
        .zip([0_usize..3, 4..5, 5..10, 10..11, 12..14].iter().cloned())
        .for_each(|(is, expect)| {
            assert_eq!(is, expect);
        });
}

#[test]
fn tokenize_boring_genetive_s() {
    let tok = tokenizer::<PathBuf>(None).unwrap();
    let ranges = apply_tokenizer(&tok, r#"The Y's car is yellow."#);

    ranges
        .zip([0_usize..3, 4..7, 8..11].iter().cloned())
        .for_each(|(is, expect)| {
            assert_eq!(is, expect);
        });
}

#[test]
fn tokenize_foo_dot() {
    let tok = tokenizer::<PathBuf>(None).unwrap();
    let ranges = apply_tokenizer(&tok, r#"Foo."#);

    ranges
        .zip([0_usize..3, 3..4].iter().cloned())
        .for_each(|(is, expect)| {
            assert_eq!(is, expect);
        });
}

#[test]
fn tokenize_foo() {
    let tok = tokenizer::<PathBuf>(None).unwrap();
    let ranges = apply_tokenizer(&tok, r#"foo"#);

    ranges
        .zip(std::iter::once(0_usize..3))
        .for_each(|(is, expect)| {
            assert_eq!(is, expect);
        });
}

#[test]
fn tokenize_square_bracket_foo_square_bracket() {
    let text = r#"[1337]"#;
    let tok = tokenizer::<PathBuf>(None).unwrap();
    let ranges = apply_tokenizer(&tok, text);

    ranges
        .zip([0_usize..1, 1..5, 5..6].iter().cloned())
        .for_each(|(is, expect)| {
            dbg!((sub_chars(text, is.clone()), sub_chars(text, expect.clone())));
            assert_eq!(is, expect);
        });
}
