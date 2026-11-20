use crate::checker::dictaffix::is_valid_hunspell_dic;

use super::*;

#[test]
fn hunspell_dic_format() {
    const GOOD: &str = "2
whitespazes
catsndogs
";
    const BAD_1: &str = "foo
12349
bar
";
    const BAD_2: &str = "2
12349
bar
";
    const BAD_3: &str = "foo
xxx
bar
";
    assert!(is_valid_hunspell_dic(&mut GOOD.as_bytes()).is_ok());
    assert!(is_valid_hunspell_dic(&mut BAD_1.as_bytes()).is_err());
    assert!(is_valid_hunspell_dic(&mut BAD_2.as_bytes()).is_err());
    assert!(is_valid_hunspell_dic(&mut BAD_3.as_bytes()).is_err());
}

macro_rules! parametrized_vulgar_fraction_or_emoji {
        ($($name:ident: $value:expr,)*) => {
        $(
            #[test]
            fn $name() {
                let (input, expected) = $value;
                assert_eq!(expected, consists_of_vulgar_fractions_or_emojis(input));
            }
        )*
        }
    }

parametrized_vulgar_fraction_or_emoji! {
    empty: ("", false),
    emojis: ("🐍🤗🦀", true),
    contains_emojis: ("🦀acean", false),
    contains_only_unicode: ("⅔⅔⅔↉↉↉", true),
    contains_emojis_and_unicodes: ("🐍🤗⅒🦀⅔¾", true),
    no_emojis: ("no emoji string", false),
    is_number: ("123", true),
    is_latin_letter: ("a", false),
    vulgar_fraction_one_quarter_and_emojis: ("¼🤗🦀", true),
    emojis_and_vulgar_fraction_one_half: ("🤗🦀½", true),
    emojis_and_vulgar_fraction_three_quarters: ("🤗🦀¾", true),
    emojis_and_vulgar_fraction_one_seventh: ("🤗🦀⅐", true),
    emojis_and_vulgar_fraction_one_ninth: ("🤗🦀⅑", true),
    emojis_and_vulgar_fraction_one_tenth: ("🤗🦀⅒", true),
    emojis_and_vulgar_fraction_one_third: ("🤗🦀⅓", true),
    emojis_and_vulgar_fraction_two_thirds: ("🤗🦀⅔", true),
    emojis_and_vulgar_fraction_one_fifth: ("🤗🦀⅕", true),
    emojis_and_vulgar_fraction_two_fifth: ("🤗🦀⅖", true),
    emojis_and_vulgar_fraction_three_fifths: ("🤗🦀⅗", true),
    emojis_and_vulgar_fraction_four_fifths: ( "🐍⅘", true),
    emojis_and_vulgar_fraction_one_sixth: ("🐍⅙", true),
    emojis_and_vulgar_fraction_five_sixths: ("🐍⅚", true),
    emojis_and_vulgar_fraction_one_eighth: ("🦀🐍⅛", true),
    emojis_and_vulgar_fraction_three_eighths: ("🦀🐍⅜", true),
    emojis_and_vulgar_fraction_five_eights: ("🦀🐍⅝", true),
    emojis_and_vulgar_fraction_five_eighths: ("🦀🐍⅝", true),
    emojis_and_vulgar_fraction_seven_eighths: ("🦀🐍⅞", true),
    emojis_and_vulgar_fraction_zero_thirds: ("🦀🐍↉", true),
}
