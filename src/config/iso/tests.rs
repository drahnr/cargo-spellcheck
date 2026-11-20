use super::*;
use assert_matches::assert_matches;

const EXPECTED: Lang5 = Lang5 {
    lang: Language::Deu,
    country: Country::AU,
};
const S: &str = "de_AU";

#[test]
fn iso_lang_german_austria_serde() {
    assert_eq!(S.to_owned(), EXPECTED.to_string());

    assert_matches!(serde_plain::from_str::<Lang5>(S), Ok(x) => assert_eq!(EXPECTED, x));
}

#[test]
// Exercise the comparison implementations for owned and borrowed operands.
#[allow(clippy::op_ref, clippy::cmp_owned)]
fn cmp_variants() {
    assert!(EXPECTED == S);
    assert!(EXPECTED == &S);
    assert!(EXPECTED == S.to_owned());
    assert!(EXPECTED == &S.to_owned());
    assert!(&EXPECTED == S);
}
