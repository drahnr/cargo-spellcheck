#[cfg(feature = "hunspell")]
use super::interactive::UserSelection;
use super::*;
use std::convert::TryInto;

#[cfg(feature = "hunspell")]
struct SessionFixture {
    directory: PathBuf,
    dictionary: PathBuf,
    source: PathBuf,
    config: Config,
    documents: Documentation,
}

#[cfg(feature = "hunspell")]
impl SessionFixture {
    fn new(text: &str, persistent: bool) -> Self {
        // Source patches use a temporary in CWD, so keep fixtures on the same filesystem.
        let directory = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("spellcheck-session-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let dictionary = directory.join("learned.dic");
        let source = directory.join("first.md");
        fs::write(&dictionary, "0\n").unwrap();
        fs::write(&source, text).unwrap();
        let config = Config {
            nlprules: None,
            zet: None,
            spellbook: None,
            hunspell: Some(HunspellConfig {
                skip_os_lookups: true,
                extra_dictionaries: vec![dictionary.clone()],
                learning_dictionary: if persistent {
                    Some(dictionary.clone())
                } else {
                    None
                },
                ..HunspellConfig::default()
            }),
            ..Config::default()
        };
        let documents = Documentation::load_from_str(
            ContentOrigin::CommonMarkFile(source.clone()),
            text,
            true,
            false,
        );
        Self {
            directory,
            dictionary,
            source,
            config,
            documents,
        }
    }

    fn add_document(&mut self, name: &str, text: &str) -> PathBuf {
        let path = self.directory.join(name);
        fs::write(&path, text).unwrap();
        self.documents.extend(Documentation::load_from_str(
            ContentOrigin::CommonMarkFile(path.clone()),
            text,
            true,
            false,
        ));
        path
    }

    fn run(
        &mut self,
        input: impl FnMut(
            &UserPicked,
            &mut interactive::State<'_, '_>,
            usize,
            usize,
        ) -> Result<UserSelection>,
    ) -> Result<Finish> {
        let mut input = input;
        Action::Fix.run_fix_interactive_with(
            std::mem::replace(&mut self.documents, Documentation::new()),
            self.config.clone(),
            |origin, suggestions, resolutions| {
                UserPicked::select_with(origin, suggestions, resolutions, &mut input)
            },
        )
    }
}

#[cfg(feature = "hunspell")]
impl Drop for SessionFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[cfg(feature = "hunspell")]
#[test]
fn final_acceptance_is_persisted_and_loaded_next_session() {
    let mut fixture = SessionFixture::new("Zzyzxqvocab", true);
    let finish = fixture
        .run(|_, _, _, _| Ok(UserSelection::AcceptWord))
        .unwrap();
    assert!(matches!(finish, Finish::MistakeCount(0)));
    assert_eq!(
        fs::read_to_string(&fixture.dictionary).unwrap(),
        "1\nZzyzxqvocab\n"
    );
    assert_eq!(fs::read_to_string(&fixture.source).unwrap(), "Zzyzxqvocab");
    let documents = Documentation::load_from_str(
        ContentOrigin::CommonMarkFile(fixture.source.clone()),
        "Zzyzxqvocab",
        true,
        false,
    );
    let checkers = Checkers::new(fixture.config.clone()).unwrap();
    for (origin, chunks) in documents.iter() {
        assert!(checkers.check(origin, chunks).unwrap().is_empty());
    }
}

#[cfg(feature = "hunspell")]
#[test]
fn acceptance_learns_the_complete_checked_word_across_markdown_spans() {
    let mut fixture = SessionFixture::new("Zzy**zx**qvocab Zzyzxqvocab", true);
    let mut prompts = 0;
    fixture
        .run(|_, _, _, _| {
            prompts += 1;
            Ok(UserSelection::AcceptWord)
        })
        .unwrap();
    assert_eq!(prompts, 1);
    assert_eq!(
        fs::read_to_string(&fixture.dictionary).unwrap(),
        "1\nZzyzxqvocab\n"
    );
    assert_eq!(
        fs::read_to_string(&fixture.source).unwrap(),
        "Zzy**zx**qvocab Zzyzxqvocab"
    );
}

#[cfg(feature = "hunspell")]
#[test]
fn unconfigured_acceptance_is_session_only() {
    let mut fixture = SessionFixture::new("Zzyzxqvocab Zzyzxqvocab", false);
    let mut prompts = 0;
    fixture
        .run(|_, _, _, _| {
            prompts += 1;
            Ok(UserSelection::AcceptWord)
        })
        .unwrap();
    assert_eq!(prompts, 1);
    assert_eq!(fs::read_to_string(&fixture.dictionary).unwrap(), "0\n");
    assert_eq!(fs::read_dir(&fixture.directory).unwrap().count(), 2);
}

#[cfg(feature = "hunspell")]
#[test]
fn quit_retains_current_picks_and_acceptances_and_stops_all_documents() {
    let mut fixture = SessionFixture::new("Zzyzxqvocab Spelchekr Thirdwrng", true);
    let later = fixture.directory.join("later.md");
    fs::write(&later, "Laterwrng").unwrap();
    fixture.documents.extend(Documentation::load_from_str(
        ContentOrigin::CommonMarkFile(later.clone()),
        "Laterwrng",
        true,
        false,
    ));
    let mut prompts = 0;
    let finish = fixture
        .run(|_, state, _, _| {
            prompts += 1;
            Ok(match prompts {
                1 => UserSelection::AcceptWord,
                2 => {
                    state.select_custom();
                    state.custom_replacement = "spellchecker".into();
                    UserSelection::Replacement(state.to_bandaid())
                }
                3 => UserSelection::Quit,
                _ => panic!("continued prompting after quit"),
            })
        })
        .unwrap();
    assert_eq!(prompts, 3);
    assert!(matches!(finish, Finish::MistakeCount(1)));
    assert_eq!(
        fs::read_to_string(&fixture.source).unwrap(),
        "Zzyzxqvocab spellchecker Thirdwrng"
    );
    assert_eq!(
        fs::read_to_string(&fixture.dictionary).unwrap(),
        "1\nZzyzxqvocab\n"
    );
    assert_eq!(fs::read_to_string(later).unwrap(), "Laterwrng");
}

#[cfg(feature = "hunspell")]
#[test]
fn abort_discards_source_edits_and_accepted_vocabulary() {
    let original = "Zzyzxqvocab Spelchekr Thirdwrng";
    let mut fixture = SessionFixture::new(original, true);
    let mut prompts = 0;
    let finish = fixture
        .run(|_, state, _, _| {
            prompts += 1;
            Ok(match prompts {
                1 => UserSelection::AcceptWord,
                2 => {
                    state.select_custom();
                    state.custom_replacement = "spellchecker".into();
                    UserSelection::Replacement(state.to_bandaid())
                }
                3 => UserSelection::Abort,
                _ => panic!("continued prompting after abort"),
            })
        })
        .unwrap();
    assert!(matches!(finish, Finish::Abort));
    assert_eq!(fs::read_to_string(&fixture.source).unwrap(), original);
    assert_eq!(fs::read_to_string(&fixture.dictionary).unwrap(), "0\n");
    assert_eq!(fs::read_dir(&fixture.directory).unwrap().count(), 2);
}

#[cfg(feature = "hunspell")]
#[test]
fn abort_in_a_later_document_discards_earlier_document_choices() {
    let original = "Zzyzxqvocab Spelchekr";
    let mut fixture = SessionFixture::new(original, true);
    let later = fixture.add_document("later.md", "Thirdwrng");
    let dictionary = fixture.dictionary.clone();
    let source = fixture.source.clone();
    let mut prompts = 0;
    let finish = fixture
        .run(|_, state, _, _| {
            assert_eq!(fs::read_to_string(&dictionary).unwrap(), "0\n");
            assert_eq!(fs::read_to_string(&source).unwrap(), original);
            prompts += 1;
            Ok(match prompts {
                1 => UserSelection::AcceptWord,
                2 => {
                    state.select_custom();
                    state.custom_replacement = "spellchecker".into();
                    UserSelection::Replacement(state.to_bandaid())
                }
                3 => UserSelection::Abort,
                _ => panic!("continued after abort"),
            })
        })
        .unwrap();
    assert_eq!(prompts, 3);
    assert!(matches!(finish, Finish::Abort));
    assert_eq!(fs::read_to_string(&fixture.source).unwrap(), original);
    assert_eq!(fs::read_to_string(later).unwrap(), "Thirdwrng");
    assert_eq!(fs::read_to_string(&fixture.dictionary).unwrap(), "0\n");
    assert_eq!(fs::read_dir(&fixture.directory).unwrap().count(), 3);
}

#[cfg(feature = "hunspell")]
#[test]
fn input_errors_discard_pending_source_and_vocabulary_changes() {
    let original = "Zzyzxqvocab Spelchekr Thirdwrng";
    let mut fixture = SessionFixture::new(original, true);
    let mut prompts = 0;
    let error = fixture
        .run(|_, state, _, _| {
            prompts += 1;
            match prompts {
                1 => Ok(UserSelection::AcceptWord),
                2 => {
                    state.select_custom();
                    state.custom_replacement = "spellchecker".into();
                    Ok(UserSelection::Replacement(state.to_bandaid()))
                }
                3 => bail!("simulated terminal input error"),
                _ => panic!("continued after input error"),
            }
        })
        .unwrap_err();
    assert!(error.to_string().contains("simulated terminal input error"));
    assert_eq!(fs::read_to_string(&fixture.source).unwrap(), original);
    assert_eq!(fs::read_to_string(&fixture.dictionary).unwrap(), "0\n");
    assert_eq!(fs::read_dir(&fixture.directory).unwrap().count(), 2);
}

#[cfg(feature = "hunspell")]
#[test]
fn source_write_failure_does_not_persist_accepted_words() {
    let mut fixture = SessionFixture::new("Zzyzxqvocab Spelchekr", true);
    let source = fixture.source.clone();
    let mut prompts = 0;
    let error = fixture
        .run(|_, state, _, _| {
            prompts += 1;
            Ok(match prompts {
                1 => UserSelection::AcceptWord,
                2 => {
                    state.select_custom();
                    state.custom_replacement = "spellchecker".into();
                    fs::remove_file(&source).unwrap();
                    UserSelection::Replacement(state.to_bandaid())
                }
                _ => panic!("unexpected extra diagnostic"),
            })
        })
        .unwrap_err();
    assert!(!error.to_string().contains("persist accepted words"));
    assert_eq!(prompts, 2);
    assert_eq!(fs::read_to_string(&fixture.dictionary).unwrap(), "0\n");
    assert_eq!(fs::read_dir(&fixture.directory).unwrap().count(), 1);
}

#[cfg(feature = "hunspell")]
#[test]
fn no_acceptances_does_not_open_or_recreate_the_learning_dictionary() {
    let mut fixture = SessionFixture::new("Zzyzxqvocab", true);
    let dictionary = fixture.dictionary.clone();
    let finish = fixture
        .run(|_, _, _, _| {
            fs::remove_file(&dictionary).unwrap();
            Ok(UserSelection::Skip)
        })
        .unwrap();
    assert!(matches!(finish, Finish::MistakeCount(0)));
    assert!(!fixture.dictionary.exists());
    assert_eq!(fs::read_to_string(&fixture.source).unwrap(), "Zzyzxqvocab");
    assert_eq!(fs::read_dir(&fixture.directory).unwrap().count(), 1);
}

#[cfg(feature = "hunspell")]
#[test]
fn loaded_config_targets_only_the_designated_dictionary_and_preserves_toml() {
    let mut fixture = SessionFixture::new("Zzyzxqvocab", true);
    let other = fixture.directory.join("other.dic");
    fs::write(&other, "1\nExisting/S\n").unwrap();
    let toml = "# Keep this repository configuration intact.\n[Hunspell]\nsearch_dirs = ['.']\nextra_dictionaries = ['other.dic', 'learned.dic']\nlearning_dictionary = 'learned.dic'\nskip_os_lookups = true\nuse_builtin = true\n";
    let config_path = fixture.directory.join("spellcheck.toml");
    fs::write(&config_path, toml).unwrap();
    fixture.config = Config::load_from(&config_path).unwrap().unwrap();
    fixture.config.nlprules = None;
    fixture.config.zet = None;
    fixture.config.spellbook = None;
    fixture
        .run(|_, _, _, _| Ok(UserSelection::AcceptWord))
        .unwrap();
    assert_eq!(
        fs::read_to_string(&fixture.dictionary).unwrap(),
        "1\nZzyzxqvocab\n"
    );
    assert_eq!(fs::read_to_string(other).unwrap(), "1\nExisting/S\n");
    assert_eq!(fs::read_to_string(config_path).unwrap(), toml);
}

#[cfg(feature = "hunspell")]
#[test]
fn skip_file_keeps_choices_and_continues_with_the_next_document() {
    let mut fixture = SessionFixture::new("Zzyzxqvocab Spelchekr Thirdwrng", true);
    let later = fixture.add_document("later.md", "Laterwrng");
    let mut prompts = 0;
    let finish = fixture
        .run(|_, state, _, _| {
            prompts += 1;
            Ok(match prompts {
                1 => UserSelection::AcceptWord,
                2 => UserSelection::SkipFile,
                3 => {
                    assert_eq!(state.suggestion.origin.as_path(), later.as_path());
                    state.select_custom();
                    state.custom_replacement = "later".into();
                    UserSelection::Replacement(state.to_bandaid())
                }
                _ => panic!("did not skip the remaining diagnostics in the first file"),
            })
        })
        .unwrap();
    assert_eq!(prompts, 3);
    assert!(matches!(finish, Finish::MistakeCount(1)));
    assert_eq!(
        fs::read_to_string(&fixture.source).unwrap(),
        "Zzyzxqvocab Spelchekr Thirdwrng"
    );
    assert_eq!(fs::read_to_string(later).unwrap(), "later");
    assert_eq!(
        fs::read_to_string(&fixture.dictionary).unwrap(),
        "1\nZzyzxqvocab\n"
    );
}

#[cfg(feature = "hunspell")]
#[test]
fn persistence_failures_are_reported() {
    let mut fixture = SessionFixture::new("Zzyzxqvocab", true);
    let path = fixture.dictionary.clone();
    let error = fixture
        .run(|_, _, _, _| {
            fs::remove_file(&path).unwrap();
            Ok(UserSelection::AcceptWord)
        })
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("Could not persist accepted words"));
}

macro_rules! verify_correction {
    ($text:literal, $bandaids:expr, $expected:literal) => {
        let mut sink: Vec<u8> = Vec::with_capacity(1024);

        apply_patches(
            $bandaids.into_iter().map(|bandaid| Patch::from(bandaid)),
            $text,
            &mut sink,
        )
        .expect("Line correction must work in unit test!");

        assert_eq!(String::from_utf8_lossy(sink.as_slice()), $expected);
    };
}

#[test]
fn patch_full() {
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    let patches = vec![
        Patch::Replace {
            replace_span: Span {
                start: LineColumn { line: 1, column: 6 },
                end: LineColumn {
                    line: 2,
                    column: 12,
                },
            },
            replacement: "& Omega".to_owned(),
        },
        Patch::Insert {
            insert_at: LineColumn { line: 3, column: 0 },
            content: "Icecream truck".to_owned(),
        },
    ];
    verify_correction!(
        r#"Alpha beta gamma
zeta eta beta.
"#,
        patches,
        r#"Alpha & Omega.
Icecream truck"#
    );
}

#[test]
fn patch_replace_1() {
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();
    let bandaids = vec![Patch::Replace {
        replace_span: (1_usize, 0..1).try_into().unwrap(),
        replacement: "Y".to_owned(),
    }];
    verify_correction!("T🐠🐠U", bandaids, "Y🐠🐠U");
}

#[test]
fn patch_replace_2() {
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();
    let bandaids = vec![Patch::Replace {
        replace_span: (1_usize, 1..3).try_into().unwrap(),
        replacement: "Y".to_owned(),
    }];
    verify_correction!("T🐠🐠U", bandaids, "TYU");
}

#[test]
fn patch_replace_3() {
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();
    let bandaids = vec![Patch::Replace {
        replace_span: (1_usize, 3..4).try_into().unwrap(),
        replacement: "Y".to_owned(),
    }];
    verify_correction!("T🐠🐠U", bandaids, "T🐠🐠Y");
}

#[test]
fn patch_injection_1() {
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    let patches = vec![Patch::Insert {
        insert_at: LineColumn {
            line: 1_usize,
            column: 0,
        },
        content: "Q".to_owned(),
    }];
    verify_correction!("A🐢C", patches, "QA🐢C");
}

#[test]
fn patch_injection_2() {
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    let patches = vec![Patch::Insert {
        insert_at: LineColumn {
            line: 1_usize,
            column: 2,
        },
        content: "Q".to_owned(),
    }];
    verify_correction!("A🐢C", patches, "A🐢QC");
}
#[test]
fn patch_injection_3() {
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    let patches = vec![Patch::Insert {
        insert_at: LineColumn {
            line: 1_usize,
            column: 3,
        },
        content: "Q".to_owned(),
    }];
    verify_correction!("A🐢C", patches, "A🐢CQ");
}
