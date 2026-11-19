//! A dictionary check with affixes, backed by `libhunspell`
//!
//! Does not check grammar, but tokenizes the documentation chunk, and checks
//! the individual tokens against the dictionary using the defined affixes. Can
//! handle multiple dictionaries.
//!
//! Rust binding panics become errors; failed handles are not reused. This does
//! not contain native faults or builds configured with `panic = "abort"`.

use super::{apply_tokenizer, Checker, Detector, Suggestion, SuggestionContext};

use crate::checker::dictaffix::is_valid_hunspell_dic_path;
use crate::config::{Lang5, WrappedRegex};
use crate::documentation::{CheckableChunk, ContentOrigin};
use crate::util::sub_chars;
use crate::Range;

use fs_err as fs;
use io::Write;
use lazy_static::lazy_static;

use nlprule::Tokenizer;
use std::io::{self};

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use hunspell_rs::{CheckResult, Hunspell};

use doc_chunks::Ignores;

use crate::errors::*;

use super::quirks::{
    compound_ranges, replacements_contain_dashed, replacements_contain_dashless, transform,
    Transformed,
};

pub(super) static BUILTIN_HUNSPELL_AFF: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/hunspell-data/en_US.aff"
));

pub(super) static BUILTIN_HUNSPELL_DIC: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/hunspell-data/en_US.dic"
));

// XXX hunspell does not provide an API for using in-memory dictionary or
// XXX affix files
// XXX https://github.com/hunspell/hunspell/issues/721
pub(super) fn cache_builtin_inner(
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

pub(super) fn cache_builtin() -> Result<(PathBuf, PathBuf)> {
    log::info!("Using builtin en_US hunspell dictionary and affix files");
    let base = directories::BaseDirs::new().expect("env HOME must be set");

    let cache_dir = base.cache_dir();
    let path_aff = cache_builtin_inner(cache_dir, "aff", BUILTIN_HUNSPELL_AFF)?;
    let path_dic = cache_builtin_inner(cache_dir, "dic", BUILTIN_HUNSPELL_DIC)?;
    Ok((path_dic, path_aff))
}

/// The value is `true` if string is made of emoji's or Unicode
/// `VULGAR FRACTION`.
pub fn consists_of_vulgar_fractions_or_emojis(word: &str) -> bool {
    lazy_static! {
        static ref VULGAR_OR_EMOJI: regex::RegexSet = regex::RegexSetBuilder::new([
            r"[\u00BC-\u00BE\u2150-\u215E-\u2189]",
            r"^[\p{Emoji}]+$"
        ])
        .case_insensitive(true)
        .build()
        .expect("REGEX grammar is human checked. qed");
    };
    VULGAR_OR_EMOJI.is_match(word)
}

fn catch_hunspell_panic<T>(operation: &str, action: impl FnOnce() -> Result<T>) -> Result<T> {
    catch_unwind(AssertUnwindSafe(action)).unwrap_or_else(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("non-string panic payload");
        bail!("Hunspell panicked while {operation}: {message}")
    })
}

// Bundled Hunspell maintains a process-global UTF-8 table and reference count.
// Serializing each handle is insufficient: construction, dictionary addition,
// and destruction all mutate this shared state.
static HUNSPELL_LIFECYCLE: Mutex<()> = Mutex::new(());

struct NativeHunspell(Option<Hunspell>);

// SAFETY: Native handles may move between threads, but must not be shared.
// Hunspell mutates internal state even during lookups. HunspellSafe keeps this
// owner behind a mutex; construction, dictionary addition, and destruction also
// take the lifecycle lock.
unsafe impl Send for NativeHunspell {}

impl Drop for NativeHunspell {
    fn drop(&mut self) {
        let _lifecycle = HUNSPELL_LIFECYCLE
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Err(error) = catch_hunspell_panic("destroying the native dictionary", || {
            drop(self.0.take());
            Ok(())
        }) {
            log::error!("{error}");
        }
    }
}

#[derive(Clone)]
struct HunspellSafe(Arc<Mutex<NativeHunspell>>);

impl HunspellSafe {
    fn new(aff: &str, dic: &str) -> Result<Self> {
        let engine = {
            let _lifecycle = HUNSPELL_LIFECYCLE
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            catch_hunspell_panic("initializing the native dictionary", || {
                Ok(Hunspell::new(aff, dic))
            })?
        };
        Ok(Self(Arc::new(Mutex::new(NativeHunspell(Some(engine))))))
    }

    fn with_engine<T>(
        &self,
        operation: &str,
        action: impl FnOnce(&mut Hunspell) -> T,
    ) -> Result<T> {
        catch_hunspell_panic(operation, || {
            // Lock inside the unwind boundary so a panicking call poisons the
            // shared handle. Never reuse potentially inconsistent native state.
            let mut engine = self.0.lock().map_err(|_| {
                eyre!("Hunspell is unavailable after an earlier panic; recreate the checker before {operation}")
            })?;
            let engine = engine
                .0
                .as_mut()
                .ok_or_else(|| eyre!("Hunspell dictionary has been destroyed"))?;
            Ok(action(engine))
        })
    }

    fn check(&self, word: &str) -> Result<CheckResult> {
        self.with_engine(&format!("checking word {word:?}"), |engine| {
            engine.check(word)
        })
    }

    fn suggest(&self, word: &str) -> Result<Vec<String>> {
        self.with_engine(&format!("suggesting replacements for {word:?}"), |engine| {
            engine.suggest(word)
        })
    }

    fn add_dictionary(&self, path: &str) -> Result<bool> {
        self.with_engine(&format!("adding dictionary {path:?}"), |engine| {
            // add_dic constructs another HashMgr and updates the global UTF-8
            // reference count. Take the handle lock before the lifecycle lock;
            // construction/destruction never acquire a handle lock under it.
            let _lifecycle = HUNSPELL_LIFECYCLE
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            engine.add_dictionary(path)
        })
    }
}

#[derive(Clone)]
pub struct HunspellCheckerInner {
    hunspell: HunspellSafe,
    transform_regex: Vec<WrappedRegex>,
    allow_concatenated: bool,
    allow_dashed: bool,
    allow_emojis: bool,
    check_footnote_references: bool,
    ignorelist: String,
}

impl HunspellCheckerInner {
    fn new(config: &<HunspellChecker as Checker>::Config) -> Result<Self> {
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

        let hunspell = HunspellSafe::new(aff, dic)?;
        is_valid_hunspell_dic_path(dic)?;
        hunspell.add_dictionary(dic)?;

        if cfg!(debug_assertions) && Lang5::en_US == lang {
            // "Test" is a valid word
            debug_assert_eq!(hunspell.check("Test")?, CheckResult::FoundInDictionary);
            // suggestion must contain the word itself if it is valid
            debug_assert!(hunspell.suggest("Test")?.contains(&"Test".to_string()));
        }

        // suggestion must contain the word itself if it is valid extra dictionary
        // be more strict about the extra dictionaries, they have to exist
        for extra_dic in config.extra_dictionaries() {
            log::debug!("Adding extra dictionary {}", extra_dic.display());
            if !extra_dic.is_file() {
                bail!("Extra dictionary {} is not a file", extra_dic.display());
            }
            is_valid_hunspell_dic_path(extra_dic)?;
            if let Some(extra_dic) = extra_dic.to_str() {
                if !hunspell.add_dictionary(extra_dic)? {
                    bail!("Failed to add extra dictionary path to context {extra_dic}");
                }
            } else {
                bail!(
                    "Failed to convert extra dictionary path to str {}",
                    extra_dic.display()
                );
            }
        }
        log::debug!("Dictionary setup completed successfully.");
        Ok(Self {
            hunspell,
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
        let inner = catch_hunspell_panic("initializing the checker", || {
            HunspellCheckerInner::new(config)
        })?;
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
            let plain = chunk.erase_cmark(&Ignores {
                footnote_references: !self.0.check_footnote_references,
            });
            log::trace!("{plain:?}");
            let txt = plain.as_str();
            let hunspell = &self.hunspell;
            let context = SuggestionContext {
                plain: &plain,
                chunk,
                origin,
                allow_concatenated: self.allow_concatenated,
                allow_dashed: self.allow_dashed,
                allow_emojis: self.allow_emojis,
            };

            'tokenization: for range in
                compound_ranges(&self.transform_regex, txt, apply_tokenizer(&self.1, txt))
            {
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
                    obtain_suggestions(&context, hunspell, word, range, &mut acc)?;
                } else {
                    match transform(&self.transform_regex[..], word.as_str(), range.clone()) {
                        Transformed::Fragments(word_fragments) => {
                            for (range, word_fragment) in word_fragments {
                                obtain_suggestions(
                                    &context,
                                    hunspell,
                                    word_fragment.to_owned(),
                                    range,
                                    &mut acc,
                                )?;
                            }
                        }
                        Transformed::Atomic((range, word)) => {
                            obtain_suggestions(
                                &context,
                                hunspell,
                                word.to_owned(),
                                range,
                                &mut acc,
                            )?;
                        }
                        Transformed::Whitelisted(_) => {}
                    }
                }
            }
        }
        Ok(acc)
    }
}

#[allow(clippy::too_many_arguments)]
fn obtain_suggestions<'s>(
    context: &SuggestionContext<'_, 's>,
    hunspell: &HunspellSafe,
    word: String,
    range: Range,
    acc: &mut Vec<Suggestion<'s>>,
) -> Result<()> {
    let SuggestionContext {
        plain,
        chunk,
        origin,
        allow_concatenated,
        allow_dashed,
        allow_emojis,
    } = *context;
    log::trace!("Checking {word} in {range:?}..");

    match hunspell.check(&word)? {
        CheckResult::MissingInDictionary => {
            log::trace!("No match for word (plain range: {range:?}): >{word}<");
            // get rid of single character suggestions
            let replacements =
                Vec::from_iter(hunspell.suggest(&word)?.into_iter().filter(|x| x.len() > 1));
            // single char suggestions tend to be useless

            log::debug!(target: "hunspell", "{word} --{{suggest}}--> {replacements:?}");

            // strings made of vulgar fraction or emoji
            if allow_emojis && consists_of_vulgar_fractions_or_emojis(&word) {
                log::trace!(target: "quirks", "Found emoji or vulgar fraction character, treating {word} as ok");
                return Ok(());
            }

            if allow_concatenated && replacements_contain_dashless(&word, replacements.as_slice()) {
                log::trace!(target: "quirks", "Found dashless word in replacement suggestions, treating {word} as ok");
                return Ok(());
            }
            if allow_dashed && replacements_contain_dashed(&word, replacements.as_slice()) {
                log::trace!(target: "quirks", "Found dashed word in replacement suggestions, treating {word} as ok");
                return Ok(());
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::checker::dictaffix::is_valid_hunspell_dic;
    use std::io::BufRead;

    use super::*;

    // Tiny independent dictionaries keep native safety regressions separate from
    // host dictionaries and built-in cache/encoding behavior.
    struct TestDictionary(PathBuf);

    impl TestDictionary {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "cargo-spellcheck-hunspell-safety-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir(&directory).unwrap();
            fs::write(directory.join("en_US.aff"), "SET UTF-8\n").unwrap();
            fs::write(directory.join("en_US.dic"), "2\nTest\nyet\n").unwrap();
            Self(directory)
        }

        fn engine(&self) -> HunspellSafe {
            HunspellSafe::new(
                self.0.join("en_US.aff").to_str().unwrap(),
                self.0.join("en_US.dic").to_str().unwrap(),
            )
            .unwrap()
        }

        fn config(&self) -> crate::config::HunspellConfig {
            crate::config::HunspellConfig {
                skip_os_lookups: true,
                use_builtin: false,
                search_dirs: vec![self.0.clone()].into(),
                ..Default::default()
            }
        }
    }

    impl Drop for TestDictionary {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn panic_boundary_preserves_results_and_reports_all_payload_types() {
        assert_eq!(catch_hunspell_panic("testing", || Ok(42)).unwrap(), 42);
        let error =
            catch_hunspell_panic::<()>("testing", || Err(eyre!("ordinary error"))).unwrap_err();
        assert_eq!(error.to_string(), "ordinary error");
        for (payload, expected) in [
            (
                Box::new("borrowed panic") as Box<dyn std::any::Any + Send>,
                "borrowed panic",
            ),
            (Box::new("owned panic".to_owned()), "owned panic"),
            (Box::new(7_usize), "non-string panic payload"),
        ] {
            let error =
                catch_hunspell_panic::<()>("testing", || std::panic::resume_unwind(payload))
                    .unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("Hunspell panicked while testing: {expected}")
            );
        }
    }

    #[test]
    fn constructor_binding_panic_is_an_error_and_does_not_block_later_initialization() {
        for (aff, dic) in [("bad\0aff", "unused.dic"), ("unused.aff", "bad\0dic")] {
            let error = HunspellSafe::new(aff, dic)
                .err()
                .expect("binding must panic on an interior NUL");
            assert!(
                error
                    .to_string()
                    .contains("Hunspell panicked while initializing"),
                "{error}"
            );
        }
        let dictionary = TestDictionary::new();
        assert_eq!(
            dictionary.engine().check("Test").unwrap(),
            CheckResult::FoundInDictionary
        );
    }

    #[test]
    fn binding_panics_poison_all_clones_but_not_other_handles() {
        let dictionary = TestDictionary::new();
        for operation in ["check", "suggest", "add_dictionary"] {
            let engine = dictionary.engine();
            let cloned = engine.clone();
            let error = match operation {
                "check" => engine.check("bad\0word").unwrap_err(),
                "suggest" => engine.suggest("bad\0word").unwrap_err(),
                "add_dictionary" => engine.add_dictionary("bad\0word").unwrap_err(),
                _ => unreachable!(),
            };
            assert!(
                error.to_string().contains("Hunspell panicked while"),
                "{error}"
            );
            assert!(error.to_string().contains("bad\\0word"), "{error}");
            assert!(engine.0.is_poisoned());
            for error in [
                cloned.check("Test").unwrap_err(),
                cloned.suggest("Test").unwrap_err(),
                cloned.add_dictionary("unused.dic").unwrap_err(),
            ] {
                assert!(
                    error
                        .to_string()
                        .contains("unavailable after an earlier panic"),
                    "{error}"
                );
            }
            drop(engine);
            drop(cloned);
            assert_eq!(
                dictionary.engine().check("Test").unwrap(),
                CheckResult::FoundInDictionary
            );
        }
    }

    #[test]
    fn checker_propagates_poison_errors_for_plain_atomic_and_fragment_tokens() {
        let dictionary = TestDictionary::new();
        for patterns in [vec![], vec!["::"], vec!["(Test)"]] {
            let mut spelling = dictionary.config();
            spelling.quirks.transform_regex = patterns
                .iter()
                .map(|pattern| WrappedRegex::from(fancy_regex::Regex::new(pattern).unwrap()))
                .collect();
            let checkers = crate::checker::Checkers::new(crate::Config {
                hunspell: Some(spelling),
                zet: None,
                spellbook: None,
                nlprules: None,
                ..Default::default()
            })
            .unwrap();
            let hunspell = &checkers.hunspell.as_ref().unwrap().hunspell;
            let _ = hunspell.suggest("bad\0word").unwrap_err();
            let documents = crate::Documentation::load_from_str(
                ContentOrigin::TestEntityRust,
                "/// Test\nstruct Example;",
                true,
                false,
            );
            let (origin, chunks) = documents.iter().next().unwrap();
            let error = checkers.check(origin, chunks).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("unavailable after an earlier panic"),
                "{patterns:?}: {error}"
            );
        }
    }

    #[test]
    fn shared_handle_operations_are_serialized() {
        let dictionary = TestDictionary::new();
        let engine = dictionary.engine();
        let active = std::sync::atomic::AtomicUsize::new(0);
        let maximum = std::sync::atomic::AtomicUsize::new(0);
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|threads| {
            for _ in 0..8 {
                let handle = engine.clone();
                let active = &active;
                let maximum = &maximum;
                let barrier = &barrier;
                threads.spawn(move || {
                    barrier.wait();
                    handle
                        .with_engine("testing serialization", |_| {
                            let count =
                                active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                            maximum.fetch_max(count, std::sync::atomic::Ordering::SeqCst);
                            std::thread::sleep(std::time::Duration::from_millis(2));
                            active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        })
                        .unwrap();
                    for _ in 0..16 {
                        assert_eq!(
                            handle.check("Test").unwrap(),
                            CheckResult::FoundInDictionary
                        );
                        assert!(handle.suggest("Test").unwrap().contains(&"Test".to_owned()));
                    }
                });
            }
        });
        assert_eq!(maximum.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn concurrent_construction_and_drop_preserve_live_handles() {
        let dictionary = TestDictionary::new();
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|threads| {
            for _ in 0..8 {
                let dictionary = &dictionary;
                let barrier = &barrier;
                threads.spawn(move || {
                    barrier.wait();
                    for _ in 0..32 {
                        let live = dictionary.engine();
                        drop(dictionary.engine());
                        for word in ["yet", "Yet", "YET"] {
                            assert_eq!(live.check(word).unwrap(), CheckResult::FoundInDictionary);
                        }
                        assert!(live.suggest("Test").unwrap().contains(&"Test".to_owned()));
                    }
                });
            }
        });
    }

    #[test]
    fn concurrent_dictionary_addition_and_lifecycles_preserve_live_handles() {
        let dictionary = TestDictionary::new();
        let extra = dictionary.0.join("extra.dic");
        fs::write(&extra, "1\nextra\n").unwrap();
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|threads| {
            for worker in 0..8 {
                let dictionary = &dictionary;
                let extra = &extra;
                let barrier = &barrier;
                threads.spawn(move || {
                    barrier.wait();
                    for _ in 0..32 {
                        let live = dictionary.engine();
                        if worker < 4 {
                            assert_eq!(
                                live.check("extra").unwrap(),
                                CheckResult::MissingInDictionary
                            );
                            for _ in 0..4 {
                                assert!(live.add_dictionary(extra.to_str().unwrap()).unwrap());
                                assert_eq!(
                                    live.check("extra").unwrap(),
                                    CheckResult::FoundInDictionary
                                );
                                std::thread::yield_now();
                            }
                        } else {
                            for _ in 0..4 {
                                drop(dictionary.engine());
                                std::thread::yield_now();
                            }
                        }
                        for word in ["yet", "Yet", "YET"] {
                            assert_eq!(live.check(word).unwrap(), CheckResult::FoundInDictionary);
                        }
                        assert!(live.suggest("Test").unwrap().contains(&"Test".to_owned()));
                    }
                });
            }
        });
    }

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

        let hunspell = HunspellSafe::new(
            aff.display().to_string().as_str(),
            dic.display().to_string().as_str(),
        )
        .unwrap();
        let cwd = crate::traverse::cwd().unwrap();
        let extra = dbg!(cwd.join(".config/lingo.dic"));
        assert!(extra.is_file());
        assert!(is_valid_hunspell_dic_path(&dic).is_ok());

        assert!(hunspell
            .add_dictionary(dbg!(extra.display().to_string().as_str()))
            .unwrap());

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
            assert_eq!(
                hunspell.check(word).unwrap(),
                CheckResult::FoundInDictionary
            );
            // Technically suggestion must contain the word itself if it is valid
            let suggestions = hunspell.suggest(word).unwrap();
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
