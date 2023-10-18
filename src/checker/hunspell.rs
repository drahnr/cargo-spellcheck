//! A dictionary check with affixes, backed by `libhunspell`
//!
//! Does not check grammar, but tokenizes the documentation chunk, and checks
//! the individual tokens against the dictionary using the defined affixes. Can
//! handle multiple dictionaries.

use super::{apply_tokenizer, Checker, Detector, Suggestion};

use crate::config::{Lang5, WrappedRegex};
use crate::documentation::{CheckableChunk, ContentOrigin, PlainOverlay};
use crate::util::sub_chars;
use crate::Range;

use fs_err as fs;
use io::Write;
use lazy_static::lazy_static;

use nlprule::Tokenizer;
use std::io::{self, BufRead};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hunspell_rs::{CheckResult, Hunspell};

use crate::errors::*;

use super::quirks::{
    replacements_contain_dashed, replacements_contain_dashless, transform, Transformed,
};

static BUILTIN_HUNSPELL_AFF: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/hunspell-data/en_US.aff"
));

static BUILTIN_HUNSPELL_DIC: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/hunspell-data/en_US.dic"
));

// XXX hunspell does not provide an API for using in-memory dictionary or
// XXX affix files
// XXX https://github.com/hunspell/hunspell/issues/721
fn cache_builtin_inner(
    cache_dir: impl AsRef<Path>,
    extension: &'static str,
    data: &[u8],
) -> Result<PathBuf> {
    let path = cache_dir.as_ref().join(format!(
        "cargo-spellcheck/{}/{}.{}",
        env!("CARGO_PKG_VERSION"),
        "en_US",
        extension
    ));
    fs::create_dir_all(path.parent().unwrap())?;
    // check if file exists
    if let Ok(f) = fs::File::open(&path) {
        // in case somebody else is currently writing to it
        // wait for that to complete
        let flock = fd_lock::RwLock::new(f);
        let _ = flock.read()?;
        return Ok(path);
    }
    let f = fs::OpenOptions::new()
        .truncate(true)
        .create(true)
        .write(true)
        .open(&path)?;
    let mut flock = fd_lock::RwLock::new(f);
    // if there are multiple instances, allow the first to write it all
    if let Ok(mut f) = flock.try_write() {
        f.write_all(data)?;
        return Ok(path);
    }

    // .. but block execution until the first completed so
    // there are no cases of partial data
    let _ = flock.read()?;

    Ok(path)
}

fn cache_builtin() -> Result<(PathBuf, PathBuf)> {
    log::info!("Using builtin en_US hunspell dictionary and affix files");
    let base = directories::BaseDirs::new().expect("env HOME must be set");

    let cache_dir = base.cache_dir();
    let path_aff = cache_builtin_inner(&cache_dir, "aff", BUILTIN_HUNSPELL_AFF)?;
    let path_dic = cache_builtin_inner(&cache_dir, "dic", BUILTIN_HUNSPELL_DIC)?;
    Ok((path_dic, path_aff))
}

/// The value is `true` if string is made of emoji's or Unicode
/// `VULGAR FRACTION`.
pub fn consists_of_vulgar_fractions_or_emojis(word: &str) -> bool {
    lazy_static! {
        static ref VULGAR_OR_EMOJI: regex::RegexSet = regex::RegexSetBuilder::new(&[
            r"[\u00BC-\u00BE\u2150-\u215E-\u2189]",
            r"^[\p{Emoji}]+$"
        ])
        .case_insensitive(true)
        .build()
        .expect("REGEX grammar is human checked. qed");
    };
    return VULGAR_OR_EMOJI.is_match(word);
}

#[derive(Clone)]
struct HunspellSafe(Arc<Hunspell>);

unsafe impl Send for HunspellSafe {}
// We only use it in RO so it's ok.
unsafe impl Sync for HunspellSafe {}

impl std::ops::Deref for HunspellSafe {
    type Target = Hunspell;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Hunspell> for HunspellSafe {
    fn from(hunspell: Hunspell) -> Self {
        Self(Arc::new(hunspell))
    }
}

#[derive(Clone)]
pub struct HunspellCheckerInner {
    hunspell: HunspellSafe,
    transform_regex: Vec<WrappedRegex>,
    allow_concatenated: bool,
    allow_dashed: bool,
    allow_emojis: bool,
    ignorelist: String,
}

impl HunspellCheckerInner {
    fn new(config: &<HunspellChecker as Checker>::Config) -> Result<Self> {
        // TODO allow override
        let (transform_regex, allow_concatenated, allow_dashed, allow_emojis) = {
            let quirks = &config.quirks;
            {
                (
                    quirks.transform_regex().to_vec(),
                    quirks.allow_concatenated(),
                    quirks.allow_dashed(),
                    quirks.allow_emojis(),
                )
            }
        };
        // FIXME rename the config option
        let ignorelist = config.tokenization_splitchars.clone();
        // without these, a lot of those would be flagged as mistakes.
        debug_assert!(ignorelist.contains(','));
        debug_assert!(ignorelist.contains('.'));
        debug_assert!(ignorelist.contains(';'));
        debug_assert!(ignorelist.contains('!'));
        debug_assert!(ignorelist.contains('?'));

        // setup hunspell:
        let search_dirs = config.search_dirs();

        let lang = config.lang().to_string();
        let lang = lang.as_str();

        // lookup paths are really just an attempt to provide a dictionary, so be more forgiving
        // when encountering errors here
        let (dic, aff): (PathBuf, PathBuf) = search_dirs
            .into_iter()
            .filter(|search_dir| {
                let keep = search_dir.is_dir();
                if !keep {
                    // search_dir also contains the default paths, so just silently ignore these
                    log::debug!(
                        "Dictionary search path is not a directory {}",
                        search_dir.display()
                    );
                } else {
                    log::debug!(
                        "Found dictionary search path {}",
                        search_dir.display()
                    );
                }
                keep
            })
            .find_map(|search_dir| {
                let dic = search_dir.join(lang).with_extension("dic");
                if !dic.is_file() {
                    log::debug!(
                        "Dictionary path dervied from search dir is not a file {}",
                        dic.display()
                    );
                    return None;
                }
                let aff = search_dir.join(lang).with_extension("aff");
                if !aff.is_file() {
                    log::debug!(
                        "Affixes path dervied from search dir is not a file {}",
                        aff.display()
                    );
                    return None;
                }
                log::debug!("Using dic {} and aff {}", dic.display(), aff.display());
                Some((dic, aff))
            })
            .ok_or_else(|| {
                eyre!("Failed to find any {lang}.dic / {lang}.aff in any search dir or no search provided",
                    lang = lang)
            })
            .or_else(|e| {
                if config.use_builtin {
                    Ok(cache_builtin()?)
                } else {
                    Err(e)
                }
            })?;

        let dic = dic.to_str().unwrap();
        let aff = aff.to_str().unwrap();

        let mut hunspell = Hunspell::new(aff, dic);
        is_valid_hunspell_dic_path(dic)?;
        hunspell.add_dictionary(dic);

        if cfg!(debug_assertions) && Lang5::en_US == lang {
            // "Test" is a valid word
            debug_assert_eq!(hunspell.check("Test"), CheckResult::FoundInDictionary);
            // suggestion must contain the word itself if it is valid
            debug_assert!(hunspell.suggest("Test").contains(&"Test".to_string()));
        }

        // suggestion must contain the word itself if it is valid extra dictionary
        // be more strict about the extra dictionaries, they have to exist
        for extra_dic in config.extra_dictionaries() {
            log::debug!("Adding extra dictionary {}", extra_dic.display());
            if !extra_dic.is_file() {
                bail!("Extra dictionary {} is not a file", extra_dic.display())
            }
            is_valid_hunspell_dic_path(extra_dic)?;
            if let Some(extra_dic) = extra_dic.to_str() {
                if !hunspell.add_dictionary(extra_dic) {
                    bail!("Failed to add extra dictionary path to context {extra_dic}")
                }
            } else {
                bail!(
                    "Failed to convert extra dictionary path to str {}",
                    extra_dic.display()
                )
            }
        }
        log::debug!("Dictionary setup completed successfully.");
        Ok(Self {
            hunspell: HunspellSafe::from(hunspell),
            transform_regex,
            allow_concatenated,
            allow_dashed,
            allow_emojis,
            ignorelist,
        })
    }
}

#[derive(Clone)]
pub struct HunspellChecker(pub Arc<HunspellCheckerInner>, pub Arc<Tokenizer>);

impl std::ops::Deref for HunspellChecker {
    type Target = HunspellCheckerInner;
    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl HunspellChecker {
    pub fn new(config: &<HunspellChecker as Checker>::Config) -> Result<Self> {
        let tokenizer = super::tokenizer::<&PathBuf>(None)?;
        let inner = HunspellCheckerInner::new(config)?;
        let hunspell = Arc::new(inner);
        Ok(HunspellChecker(hunspell, tokenizer))
    }
}

impl Checker for HunspellChecker {
    type Config = crate::config::HunspellConfig;

    fn detector() -> Detector {
        Detector::Hunspell
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
            let plain = chunk.erase_cmark();
            log::trace!("{plain:?}");
            let txt = plain.as_str();
            let hunspell = &*self.hunspell.0;

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
                        &hunspell,
                        &origin,
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
                                    &hunspell,
                                    &origin,
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
                                &hunspell,
                                &origin,
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
    hunspell: &Hunspell,
    origin: &ContentOrigin,
    word: String,
    range: Range,
    allow_concatenated: bool,
    allow_dashed: bool,
    allow_emojis: bool,
    acc: &mut Vec<Suggestion<'s>>,
) {
    match hunspell.check(&word) {
        CheckResult::MissingInDictionary => {
            log::trace!("No match for word (plain range: {range:?}): >{word}<");
            // get rid of single character suggestions
            let replacements = hunspell
                .suggest(&word)
                .into_iter()
                .filter(|x| x.len() > 1) // single char suggestions tend to be useless
                .collect::<Vec<_>>();

            log::debug!(target: "hunspell", "{word} --{{suggest}}--> {replacements:?}");

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
                    detector: Detector::Hunspell,
                    range,
                    span,
                    origin: origin.clone(),
                    replacements: replacements.clone(),
                    chunk,
                    description: Some("Possible spelling mistake found.".to_owned()),
                })
            }
        }
        CheckResult::FoundInDictionary => {
            log::trace!("Found a match for word (plain range: {range:?}): >{word}<",);
        }
    }
}

/// Check if provided path has valid dictionary format.
///
/// This is a YOLO check.
fn is_valid_hunspell_dic_path(path: impl AsRef<Path>) -> Result<()> {
    let reader = io::BufReader::new(fs::File::open(path.as_ref())?);
    is_valid_hunspell_dic(reader)
}

/// Check a reader for correct hunspell format.
fn is_valid_hunspell_dic(reader: impl BufRead) -> Result<()> {
    let mut iter = reader.lines().enumerate();
    if let Some((_lineno, first)) = iter.next() {
        let first = first?;
        let _ = first.parse::<u64>().wrap_err_with(|| {
            eyre!("First line of extra dictionary must a number, but is: >{first}<")
        })?;
    }
    // Just check the first 10 lines, don't waste much time here
    // the first two are the most important ones.
    for (lineno, line) in iter.take(10) {
        // All lines after must be format x.
        if let Ok(num) = line?.parse::<i64>() {
            bail!("Line {lineno} of extra dictionary must not be a number, but is: >{num}<",)
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn hunspell_binding_is_sane() {
        let config = crate::config::HunspellConfig::default();
        let search_dirs = config.search_dirs();

        let mut srcs = None;
        for search_dir in search_dirs {
            let dic = search_dir.join("en_US.dic");
            let aff = search_dir.join("en_US.aff");
            if dic.is_file() && aff.is_file() && is_valid_hunspell_dic_path(&dic).is_ok() {
                srcs = Some((dic, aff));
                break;
            }
        }

        let (dic, aff) = dbg!(srcs.unwrap());

        let mut hunspell = Hunspell::new(
            aff.display().to_string().as_str(),
            dic.display().to_string().as_str(),
        );
        let cwd = crate::traverse::cwd().unwrap();
        let extra = dbg!(cwd.join(".config/lingo.dic"));
        assert!(extra.is_file());
        assert!(is_valid_hunspell_dic_path(&dic).is_ok());

        hunspell.add_dictionary(dbg!(extra.display().to_string().as_str()));

        let extra_dic = io::BufReader::new(fs::File::open(extra).unwrap());
        for (lineno, line) in extra_dic.lines().enumerate().skip(1) {
            let line = line.unwrap();
            let word = if line.contains('/') {
                line.split('/').next().unwrap()
            } else {
                line.as_str()
            };

            println!("testing >{word}< against line #{lineno} >{line}<");
            // "whitespace" is a word part of our custom dictionary
            assert_eq!(hunspell.check(word), CheckResult::FoundInDictionary);
            // Technically suggestion must contain the word itself if it is valid
            let suggestions = hunspell.suggest(word);
            // but this is not true for i.e. `clang`
            // assert!(suggestions.contains(&word.to_owned()));
            if !suggestions.contains(&word.to_owned()) {
                eprintln!("suggest does not contain valid self: {word} ∉ {suggestions:?}",);
            }
        }
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
