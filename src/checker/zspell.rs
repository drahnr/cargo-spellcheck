//! A dictionary check with affixes, backed by `libhunspell`
//!
//! Does not check grammar, but tokenizes the documentation chunk, and checks
//! the individual tokens against the dictionary using the defined affixes. Can
//! handle multiple dictionaries.

use super::{apply_tokenizer, Checker, Detector, Suggestion};

use crate::checker::dictaffix::DicAff;
use crate::config::WrappedRegex;
use crate::documentation::{CheckableChunk, ContentOrigin, PlainOverlay};
use crate::util::sub_chars;
use crate::Range;

use nlprule::Tokenizer;

use std::path::PathBuf;
use std::sync::Arc;

use doc_chunks::Ignores;

use crate::errors::*;

use super::quirks::{
    replacements_contain_dashed, replacements_contain_dashless, transform, Transformed,
};

use super::hunspell::consists_of_vulgar_fractions_or_emojis;

#[derive(Clone)]
pub struct ZetCheckerInner {
    zspell: zspell::Dictionary,
    transform_regex: Vec<WrappedRegex>,
    allow_concatenated: bool,
    allow_dashed: bool,
    allow_emojis: bool,
    check_footnote_references: bool,
    ignorelist: String,
}

impl ZetCheckerInner {
    fn new(config: &<ZetChecker as Checker>::Config) -> Result<Self> {
        // TODO allow override
        let (
            transform_regex,
            allow_concatenated,
            allow_dashed,
            allow_emojis,
            check_footnote_references,
        ) = {
            let quirks = &config.quirks;
            (
                quirks.transform_regex().to_vec(),
                quirks.allow_concatenated(),
                quirks.allow_dashed(),
                quirks.allow_emojis(),
                quirks.check_footnote_references(),
            )
        };
        // FIXME rename the config option
        let ignorelist = config.tokenization_splitchars.clone();
        // without these, a lot of those would be flagged as mistakes.
        debug_assert!(ignorelist.contains(','));
        debug_assert!(ignorelist.contains('.'));
        debug_assert!(ignorelist.contains(';'));
        debug_assert!(ignorelist.contains('!'));
        debug_assert!(ignorelist.contains('?'));

        let DicAff { dic, aff } = DicAff::load(
            &config.extra_dictionaries[..],
            &config.search_dirs,
            config.lang(),
            config.use_builtin,
            config.skip_os_lookups,
        )?;

        let aff = aff.replace("UTF8", "UTF-8");
        let zet = zspell::builder().config_str(&aff).dict_str(&dic).build()?;

        log::debug!("Dictionary setup completed successfully.");
        Ok(Self {
            zspell: zet,
            transform_regex,
            allow_concatenated,
            allow_dashed,
            allow_emojis,
            check_footnote_references,
            ignorelist,
        })
    }
}

#[derive(Clone)]
pub struct ZetChecker(pub Arc<ZetCheckerInner>, pub Arc<Tokenizer>);

impl std::ops::Deref for ZetChecker {
    type Target = ZetCheckerInner;
    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl ZetChecker {
    pub fn new(config: &<ZetChecker as Checker>::Config) -> Result<Self> {
        let tokenizer = super::tokenizer::<&PathBuf>(None)?;
        let inner = ZetCheckerInner::new(config)?;
        let hunspell = Arc::new(inner);
        Ok(ZetChecker(hunspell, tokenizer))
    }
}

impl Checker for ZetChecker {
    type Config = crate::config::ZetConfig;

    fn detector() -> Detector {
        Detector::ZSpell
    }

    fn check<'a, 's>(
        &self,
        origin: &ContentOrigin,
        chunks: &'a [CheckableChunk],
    ) -> Result<Vec<Suggestion<'s>>>
    where
        'a: 's,
    {
        let mut acc = Vec::with_capacity(chunks.len());

        for chunk in chunks {
            let plain = chunk.erase_cmark(&Ignores {
                footnote_references: !self.0.check_footnote_references,
            });
            log::trace!("{plain:?}");
            let txt = plain.as_str();

            'tokenization: for range in apply_tokenizer(&self.1, txt) {
                let word = sub_chars(txt, range.clone());
                if range.len() == 1
                    && word
                        .chars()
                        .next()
                        .filter(|c| self.ignorelist.contains(*c))
                        .is_some()
                {
                    continue 'tokenization;
                }
                if self.transform_regex.is_empty() {
                    obtain_suggestions(
                        &plain,
                        chunk,
                        &self.zspell,
                        origin,
                        word,
                        range,
                        self.allow_concatenated,
                        self.allow_dashed,
                        self.allow_emojis,
                        &mut acc,
                    )
                } else {
                    match transform(&self.transform_regex[..], word.as_str(), range.clone()) {
                        Transformed::Fragments(word_fragments) => {
                            for (range, word_fragment) in word_fragments {
                                obtain_suggestions(
                                    &plain,
                                    chunk,
                                    &self.zspell,
                                    origin,
                                    word_fragment.to_owned(),
                                    range,
                                    self.allow_concatenated,
                                    self.allow_dashed,
                                    self.allow_emojis,
                                    &mut acc,
                                );
                            }
                        }
                        Transformed::Atomic((range, word)) => {
                            obtain_suggestions(
                                &plain,
                                chunk,
                                &self.zspell,
                                origin,
                                word.to_owned(),
                                range,
                                self.allow_concatenated,
                                self.allow_dashed,
                                self.allow_emojis,
                                &mut acc,
                            );
                        }
                        Transformed::Whitelisted(_) => {}
                    }
                }
            }
        }
        Ok(acc)
    }
}

fn obtain_suggestions<'s>(
    plain: &PlainOverlay,
    chunk: &'s CheckableChunk,
    zspell: &zspell::Dictionary,
    origin: &ContentOrigin,
    word: String,
    range: Range,
    allow_concatenated: bool,
    allow_dashed: bool,
    allow_emojis: bool,
    acc: &mut Vec<Suggestion<'s>>,
) {
    log::trace!("Checking {word} in {range:?}..");

    match zspell.check_word(&word) {
        false => {
            log::trace!("No match for word (plain range: {range:?}): >{word}<");
            // get rid of single character suggestions
            let replacements = vec![];
            // single char suggestions tend to be useless

            log::debug!(target: "zspell", "{word} --{{suggest}}--> {replacements:?}");

            // strings made of vulgar fraction or emoji
            if allow_emojis && consists_of_vulgar_fractions_or_emojis(&word) {
                log::trace!(target: "quirks", "Found emoji or vulgar fraction character, treating {word} as ok");
                return;
            }

            if allow_concatenated && replacements_contain_dashless(&word, replacements.as_slice()) {
                log::trace!(target: "quirks", "Found dashless word in replacement suggestions, treating {word} as ok");
                return;
            }
            if allow_dashed && replacements_contain_dashed(&word, replacements.as_slice()) {
                log::trace!(target: "quirks", "Found dashed word in replacement suggestions, treating {word} as ok");
                return;
            }
            for (range, span) in plain.find_spans(range.clone()) {
                acc.push(Suggestion {
                    detector: Detector::ZSpell,
                    range,
                    span,
                    origin: origin.clone(),
                    replacements: replacements.clone(),
                    chunk,
                    description: Some("Possible spelling mistake found.".to_owned()),
                })
            }
        }
        true => {
            log::trace!("Found a match for word (plain range: {range:?}): >{word}<",);
        }
    }
}

#[cfg(test)]
mod tests {
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
}
