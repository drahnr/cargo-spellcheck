use crate::checker::dictaffix::is_valid_hunspell_dic;
use fs_err as fs;
use std::io::{self, BufRead};

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
    let error = catch_hunspell_panic::<()>("testing", || Err(eyre!("ordinary error"))).unwrap_err();
    assert_eq!(error.to_string(), "ordinary error");
    for (payload, expected) in [
        (
            Box::new("borrowed panic") as Box<dyn std::any::Any + Send>,
            "borrowed panic",
        ),
        (Box::new("owned panic".to_owned()), "owned panic"),
        (Box::new(7_usize), "non-string panic payload"),
    ] {
        let error = catch_hunspell_panic::<()>("testing", || std::panic::resume_unwind(payload))
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
                    .run_action("testing serialization", |_| {
                        let count = active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
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
