use super::*;
use fs_err as fs;
use std::path::PathBuf;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "cargo-spellcheck-dictionary-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn words(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|word| (*word).to_owned()).collect()
}

#[test]
fn accepts_literal_words_without_changing_case() {
    let dir = TempDir::new();
    let file = DictionaryFile::new(dir.0.join("missing.dic"));
    let store: &dyn DictionaryStore = &file;
    for word in [
        "Word",
        "word",
        "naïve",
        "don't",
        "well-known",
        "C++",
        "東京",
    ] {
        validate_word(word).unwrap();
        store.validate_word(word).unwrap();
    }
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
}

#[test]
fn rejects_dictionary_syntax_and_multiword_input() {
    let dir = TempDir::new();
    let file = DictionaryFile::new(dir.0.join("missing.dic"));
    let store: &dyn DictionaryStore = &file;
    for word in [
        "",
        "123",
        "-12",
        "+12",
        "two words",
        " leading",
        "trailing ",
        "word\tpo:noun",
        "word\n",
        "word\r",
        "word\0",
        "word/S",
        "word\\/S",
        "#comment",
        "\u{feff}word",
        "two\u{a0}words",
    ] {
        assert!(validate_word(word).is_err(), "accepted {:?}", word);
        assert!(store.validate_word(word).is_err(), "accepted {:?}", word);
    }
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
}

#[test]
fn empty_batch_does_no_io() {
    let dir = TempDir::new();
    let file = DictionaryFile::new(dir.0.join("missing.dic"));
    let store: &dyn DictionaryStore = &file;
    store.update(&BTreeSet::new()).unwrap();
    store.update_entries(&BTreeSet::new()).unwrap();
    persist_entries(&dir.0.join("also-missing.dic"), &BTreeSet::new()).unwrap();
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
}

#[test]
fn invalid_batch_does_not_touch_dictionary_or_create_lock() {
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    let original = "1\nExisting/S\n";
    fs::write(&path, original).unwrap();
    let file = DictionaryFile::new(path.clone());
    let store: &dyn DictionaryStore = &file;
    assert!(store
        .update(&words(&["Accepted", "invalid/flags"]))
        .is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[test]
fn read_returns_stems_without_rewriting_dictionary_or_creating_lock() {
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    let file = DictionaryFile::new(path.clone());
    let store: &dyn DictionaryStore = &file;
    for (original, expected) in [
        ("0", BTreeSet::new()),
        ("0\n", BTreeSet::new()),
        ("0\r\n", BTreeSet::new()),
        (
            "99\r\nWord/SM\r\nWord\r\nword/AB\tpo:noun\r\nÉclair po:noun\r\n東京",
            words(&["Word", "word", "Éclair", "東京"]),
        ),
    ] {
        fs::write(&path, original).unwrap();
        assert_eq!(store.read().unwrap(), expected);
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }
}

#[test]
fn read_entries_returns_exact_tokens_without_morphology_or_io_side_effects() {
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    let file = DictionaryFile::new(path.clone());
    let store: &dyn DictionaryStore = &file;
    for (original, expected) in [
        ("0", BTreeSet::new()),
        ("0\n", BTreeSet::new()),
        ("0\r\n", BTreeSet::new()),
        (
            concat!(
                "99\r\n",
                "Word/SM\r\n",
                "Word/MS po:noun\r\n",
                "Word/SM\tpo:verb\r\n",
                "Word\tpo:noun st:word/metadata\r\n",
                "word/S\r\n",
                "Éclair po:noun\r\n",
                "東京",
            ),
            words(&["Word/SM", "Word/MS", "Word", "word/S", "Éclair", "東京"]),
        ),
    ] {
        fs::write(&path, original).unwrap();
        assert_eq!(store.read_entries().unwrap(), expected);
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }
}

#[test]
fn plain_entries_share_the_legacy_merge_behavior() {
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    let legacy = dir.0.join("legacy.dic");
    let original = "99\r\nWord/S\r\nOther\tpo:noun\r\n";
    fs::write(&path, original).unwrap();
    fs::write(&legacy, original).unwrap();
    let accepted = words(&["Word", "Other", "New", "word"]);
    let entries = accepted
        .iter()
        .cloned()
        .map(DictionaryEntry::plain)
        .collect::<Result<BTreeSet<_>>>()
        .unwrap();
    let file = DictionaryFile::new(path.clone());
    let store: &dyn DictionaryStore = &file;

    store.update_entries(&entries).unwrap();
    persist_words(&legacy, &accepted).unwrap();

    assert_eq!(fs::read(&path).unwrap(), fs::read(&legacy).unwrap());
    assert_eq!(
        store.read_entries().unwrap(),
        words(&["Word/S", "Other", "New", "Word", "word"])
    );
    assert_eq!(store.read().unwrap(), accepted);
}

#[test]
fn same_store_rereads_external_edits_and_merges_updates() {
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    fs::write(&path, "1\nExisting/S\n").unwrap();
    let file = DictionaryFile::new(path.clone());
    let store: &dyn DictionaryStore = &file;
    assert_eq!(store.read().unwrap(), words(&["Existing"]));
    store.update(&words(&["First"])).unwrap();
    assert_eq!(store.read().unwrap(), words(&["Existing", "First"]));

    fs::write(&path, "1\nEdited/SM\n").unwrap();
    assert_eq!(store.read().unwrap(), words(&["Edited"]));
    let external = dir.0.join("external.dic");
    fs::write(&external, "17\r\nExternal/SM\tpo:noun\r\n").unwrap();
    fs::rename(&external, &path).unwrap();
    assert_eq!(store.read().unwrap(), words(&["External"]));
    store.update(&words(&["External", "Second"])).unwrap();
    assert_eq!(store.read().unwrap(), words(&["External", "Second"]));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "3\r\nExternal/SM\tpo:noun\r\nExternal\r\nSecond\r\n"
    );
}

#[test]
fn failed_read_does_not_return_cached_words_or_create_files() {
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    let file = DictionaryFile::new(path.clone());
    let store: &dyn DictionaryStore = &file;
    assert!(store.read().is_err());
    assert!(store.read_entries().is_err());
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
    fs::write(&path, "1\nExisting/S\n").unwrap();
    assert_eq!(store.read().unwrap(), words(&["Existing"]));
    assert_eq!(store.read_entries().unwrap(), words(&["Existing/S"]));
    fs::remove_file(&path).unwrap();
    assert!(store.read().is_err());
    assert!(store.read_entries().is_err());
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
    fs::create_dir(&path).unwrap();
    assert!(store.read().is_err());
    assert!(store.read_entries().is_err());
    assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
}

#[test]
fn merge_preserves_flags_order_morphology_and_case_and_recounts() {
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    fs::write(&path, "99\nZebra/SM\nApple\tpo:noun\nPear po:noun").unwrap();
    let accepted = words(&["Zebra", "Apple", "Pear", "apple", "Beta", "Alpha"]);
    persist_words(&path, &accepted).unwrap();
    let expected = "7\nZebra/SM\nApple\tpo:noun\nPear po:noun\nAlpha\nBeta\nZebra\napple\n";
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    persist_words(&path, &accepted).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    // Only the destination and stable sidecar remain after replacement.
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn duplicates_only_batch_does_not_rewrite_dictionary() {
    use std::os::unix::fs::MetadataExt;

    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    let link = dir.0.join("original.dic");
    let original = "3\nWord/S\nWord\nword\n";
    fs::write(&path, original).unwrap();
    fs::hard_link(&path, &link).unwrap();
    let before = fs::metadata(&link).unwrap();

    persist_words(&path, &words(&["Word", "word"])).unwrap();

    // The hard link keeps the original inode alive, ruling out inode reuse
    // if a needless atomic replacement happened to write identical bytes.
    let after = fs::metadata(&path).unwrap();
    assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert_eq!(fs::read_to_string(&link).unwrap(), original);
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 3);
}

#[test]
fn next_session_rereads_an_external_replacement() {
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    fs::write(&path, "1\nExisting/S\n").unwrap();
    persist_words(&path, &words(&["First"])).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "2\nExisting/S\nFirst\n");

    let external = dir.0.join("external.dic");
    fs::write(&external, "17\nExisting/S\nFirst\nExternal/SM\tpo:noun\n").unwrap();
    fs::rename(&external, &path).unwrap();
    persist_words(&path, &words(&["First", "External", "Second"])).unwrap();

    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "5\nExisting/S\nFirst\nExternal/SM\tpo:noun\nExternal\nSecond\n"
    );
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[test]
fn flagged_only_acceptance_adds_plain_entries_and_preserves_existing_entries() {
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    let file = DictionaryFile::new(path.clone());
    let store: &dyn DictionaryStore = &file;
    let original = concat!(
        "10\n",
        "Compound/c\tpo:noun\n",
        "Forbidden/!\tpo:noun\n",
        "Flagged/SM po:noun\n",
        "Mixed/SM\n",
        "Mixed\n",
        "Tabbed/SM\n",
        "Tabbed\tpo:noun st:tab/metadata\n",
        "Spaced/SM\n",
        "Spaced po:noun st:space/metadata\n",
        "Untouched/c\n",
    );
    let expected = concat!(
        "13\n",
        "Compound/c\tpo:noun\n",
        "Forbidden/!\tpo:noun\n",
        "Flagged/SM po:noun\n",
        "Mixed/SM\n",
        "Mixed\n",
        "Tabbed/SM\n",
        "Tabbed\tpo:noun st:tab/metadata\n",
        "Spaced/SM\n",
        "Spaced po:noun st:space/metadata\n",
        "Untouched/c\n",
        "Compound\n",
        "Flagged\n",
        "Forbidden\n",
    );
    let accepted = words(&[
        "Compound",
        "Forbidden",
        "Flagged",
        "Mixed",
        "Tabbed",
        "Spaced",
    ]);
    let mut snapshot = accepted.clone();
    snapshot.insert("Untouched".to_owned());
    for newline in ["\n", "\r\n"] {
        fs::write(&path, original.replace('\n', newline)).unwrap();
        assert_eq!(store.read().unwrap(), snapshot);
        store.update(&accepted).unwrap();
        let expected = expected.replace('\n', newline);
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
        assert_eq!(store.read().unwrap(), snapshot);
        store.update(&accepted).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
    }
}

#[test]
fn preserves_flagged_and_unflagged_entries_but_deduplicates_new_stems() {
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    fs::write(&path, "4\nWord/SM\nWord\nWord/AB\tpo:noun\nOther po:noun\n").unwrap();

    persist_words(&path, &words(&["Word", "Other", "NewWord"])).unwrap();

    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "5\nWord/SM\nWord\nWord/AB\tpo:noun\nOther po:noun\nNewWord\n"
    );
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[test]
fn unicode_and_case_roundtrip_without_normalization() {
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    fs::write(&path, "3\nÉclair/S\n東京\nWord\n").unwrap();
    let accepted = words(&[
        "Éclair",
        "éclair",
        "東京",
        "Word",
        "word",
        "naïve",
        "café",
        "cafe\u{301}",
    ]);
    let expected = "9\nÉclair/S\n東京\nWord\ncafe\u{301}\ncafé\nnaïve\nword\nÉclair\néclair\n";

    persist_words(&path, &accepted).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    persist_words(&path, &accepted).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    persist_words(&path, &words(&["東京", "Ωmega"])).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "10\nÉclair/S\n東京\nWord\ncafe\u{301}\ncafé\nnaïve\nword\nÉclair\néclair\nΩmega\n"
    );
}

#[test]
fn header_only_and_unterminated_dictionaries_get_complete_entries() {
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    for (original, expected) in [
        ("0", "1\nNewWord\n"),
        ("0\n", "1\nNewWord\n"),
        ("0\r\n", "1\r\nNewWord\r\n"),
        ("1\nExisting", "2\nExisting\nNewWord\n"),
        ("1\r\nExisting", "2\r\nExisting\r\nNewWord\r\n"),
    ] {
        fs::write(&path, original).unwrap();
        persist_words(&path, &words(&["NewWord"])).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            expected,
            "input: {:?}",
            original
        );
        persist_words(&path, &words(&["NewWord"])).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            expected,
            "reload: {:?}",
            original
        );
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
    }
}

#[test]
fn last_accepted_word_is_a_complete_reloadable_entry() {
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    fs::write(&path, "0\n").unwrap();
    persist_words(&path, &words(&["LastAcceptedWord"])).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "1\nLastAcceptedWord\n");
    persist_words(&path, &words(&["AnotherWord"])).unwrap();
    let reloaded = fs::read_to_string(&path).unwrap();
    let mut lines = reloaded.lines();
    let count: usize = lines.next().unwrap().parse().unwrap();
    let entries = lines.collect::<Vec<_>>();
    assert_eq!(count, entries.len());
    assert_eq!(entries, ["LastAcceptedWord", "AnotherWord"]);
}

#[test]
fn preserves_crlf_entry_lines() {
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    fs::write(&path, "1\r\nOriginal/S\r\n").unwrap();
    persist_words(&path, &words(&["NewWord"])).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "2\r\nOriginal/S\r\nNewWord\r\n"
    );
}

#[cfg(unix)]
#[test]
fn preserves_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    fs::write(&path, "0\n").unwrap();
    fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    persist_words(&path, &words(&["Word"])).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[test]
fn malformed_dictionaries_are_not_replaced() {
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    let file = DictionaryFile::new(path.clone());
    let store: &dyn DictionaryStore = &file;
    for (original, cause) in [
        (b"".as_ref(), "Dictionary is empty"),
        (b"\nWord\n", "numeric entry count"),
        (b"Word\n", "numeric entry count"),
        (b"-1\nWord\n", "numeric entry count"),
        (b"+1\nWord\n", "numeric entry count"),
        (b" 1\nWord\n", "numeric entry count"),
        (b"1 \nWord\n", "numeric entry count"),
        (
            b"99999999999999999999999999999999999999999999999999999999999999999999999999999999\n",
            "numeric entry count",
        ),
        (b"1\n/SM\n", "entry on line 2"),
        (b"1\n\n", "entry on line 2"),
        (b"1\n Word\n", "entry on line 2"),
        (b"1\n\tpo:noun\n", "entry on line 2"),
        (b"1\nWo\0rd\n", "entry on line 2"),
        (b"1\nWo\x7frd\n", "entry on line 2"),
        (b"1\nWo\rrd\n", "entry on line 2"),
        (b"2\nValid/S\nInvalid\0\n", "entry on line 3"),
        (b"1\n\xff\n", "Could not read a UTF-8 dictionary"),
    ] {
        fs::write(&path, original).unwrap();
        let read_error = store.read().unwrap_err();
        assert!(
            format!("{:?}", read_error).contains(cause),
            "input: {:?}, read error: {:?}",
            original,
            read_error
        );
        let entry_read_error = store.read_entries().unwrap_err();
        assert!(
            format!("{:?}", entry_read_error).contains(cause),
            "input: {:?}, entry read error: {:?}",
            original,
            entry_read_error
        );
        let error = store.update(&words(&["NewWord"])).unwrap_err();
        let report = format!("{:?}", error);
        assert!(
            report.contains(cause),
            "input: {:?}, error: {}",
            original,
            report
        );
        let entries = BTreeSet::from([DictionaryEntry::plain("NewWord").unwrap()]);
        let entry_error = store.update_entries(&entries).unwrap_err();
        assert!(format!("{:?}", entry_error).contains(cause));
        assert_eq!(fs::read(&path).unwrap(), original);
        // A rejected dictionary must not leave a temporary replacement.
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
    }
    fs::remove_file(&path).unwrap();
    assert!(store.read().is_err());
    assert!(store.update(&words(&["NewWord"])).is_err());
    assert!(!path.exists());
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[test]
fn sidecar_open_failure_preserves_dictionary_without_creating_temporary() {
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    let sidecar = dir.0.join("lingo.dic.cargo-spellcheck.lock");
    let original = "1\nExisting/S\n";
    fs::write(&path, original).unwrap();
    fs::create_dir(&sidecar).unwrap();

    let file = DictionaryFile::new(path.clone());
    let store: &dyn DictionaryStore = &file;
    let error = store.update(&words(&["NewWord"])).unwrap_err();

    assert!(format!("{:?}", error).contains("Could not open the dictionary sidecar lock"));
    assert_eq!(store.read().unwrap(), words(&["Existing"]));
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert!(sidecar.is_dir());
    assert_eq!(fs::read_dir(&sidecar).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
#[ignore = "helper invoked only by concurrent_processes_merge_under_the_stable_lock"]
fn dictionary_persistence_subprocess() {
    use std::io::{Read, Write};

    let directory = match std::env::var_os("CARGO_SPELLCHECK_DICTIONARY_PROCESS_FIXTURE") {
        Some(directory) => PathBuf::from(directory),
        None => return,
    };
    let index = std::env::var("CARGO_SPELLCHECK_DICTIONARY_PROCESS_INDEX").unwrap();
    let lock_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join("lingo.dic.cargo-spellcheck.lock"))
        .unwrap();
    // The parent holds the sidecar until every child is ready. Verify that
    // its lock really excludes other processes, not merely other threads.
    assert!(matches!(
        lock_file.file().try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    let mut stdout = std::io::stdout();
    writeln!(stdout, "dictionary-worker-ready").unwrap();
    stdout.flush().unwrap();
    let mut start = [0];
    std::io::stdin().read_exact(&mut start).unwrap();
    assert_eq!(start, [1]);

    let mut accepted = words(&["Shared"]);
    accepted.extend((0..8).map(|word| format!("Word{}-{}", index, word)));
    persist_words(&directory.join("lingo.dic"), &accepted).unwrap();
}

#[cfg(unix)]
#[test]
fn concurrent_processes_merge_under_the_stable_lock() {
    use nix::fcntl::{fcntl, FcntlArg, OFlag};
    use nix::poll::{poll, PollFd, PollFlags};
    use std::io::{self, Read, Write};
    use std::os::unix::io::AsRawFd;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    struct Worker {
        child: Child,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        stdout_eof: bool,
        stderr_eof: bool,
    }

    impl Worker {
        fn new(child: Child) -> Self {
            // Establish ownership before any fallible pipe configuration.
            let worker = Self {
                child,
                stdout: Vec::new(),
                stderr: Vec::new(),
                stdout_eof: false,
                stderr_eof: false,
            };
            for fd in [
                worker.child.stdout.as_ref().unwrap().as_raw_fd(),
                worker.child.stderr.as_ref().unwrap().as_raw_fd(),
            ] {
                let flags = OFlag::from_bits_truncate(fcntl(fd, FcntlArg::F_GETFL).unwrap());
                fcntl(fd, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK)).unwrap();
            }
            worker
        }

        fn read_pipe(pipe: &mut impl Read, output: &mut Vec<u8>) -> io::Result<bool> {
            // Read only one bounded chunk per iteration so even a worker
            // continuously writing output cannot bypass the deadline.
            let mut buffer = [0; 8192];
            match pipe.read(&mut buffer) {
                Ok(0) => Ok(true),
                Ok(count) => {
                    if output.len() + count > 256 * 1024 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "worker output limit exceeded",
                        ));
                    }
                    output.extend_from_slice(&buffer[..count]);
                    Ok(false)
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    Ok(false)
                }
                Err(error) => Err(error),
            }
        }

        fn wait_for(&mut self, deadline: Instant, readiness: bool) -> io::Result<()> {
            loop {
                if !self.stdout_eof {
                    self.stdout_eof =
                        Self::read_pipe(self.child.stdout.as_mut().unwrap(), &mut self.stdout)?;
                }
                if !self.stderr_eof {
                    self.stderr_eof =
                        Self::read_pipe(self.child.stderr.as_mut().unwrap(), &mut self.stderr)?;
                }
                let marker = b"dictionary-worker-ready\n";
                if readiness
                    && self
                        .stdout
                        .windows(marker.len())
                        .any(|window| window == marker)
                {
                    return Ok(());
                }
                if let Some(status) = self.child.try_wait()? {
                    if !readiness && status.success() {
                        return Ok(());
                    }
                    return Err(io::Error::other(format!(
                        "worker exited {} while waiting for {}\nstdout: {}\nstderr: {}",
                        status,
                        if readiness { "readiness" } else { "completion" },
                        String::from_utf8_lossy(&self.stdout),
                        String::from_utf8_lossy(&self.stderr),
                    )));
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!(
                            "worker timed out waiting for {}\nstdout: {}\nstderr: {}",
                            if readiness { "readiness" } else { "completion" },
                            String::from_utf8_lossy(&self.stdout),
                            String::from_utf8_lossy(&self.stderr),
                        ),
                    ));
                }
                let mut pipes = Vec::new();
                if !self.stdout_eof {
                    pipes.push(PollFd::new(
                        self.child.stdout.as_ref().unwrap().as_raw_fd(),
                        PollFlags::POLLIN,
                    ));
                }
                if !self.stderr_eof {
                    pipes.push(PollFd::new(
                        self.child.stderr.as_ref().unwrap().as_raw_fd(),
                        PollFlags::POLLIN,
                    ));
                }
                // Recheck exit status even if a regressed worker closes its
                // pipes without exiting. EOF pipes are excluded to avoid spinning.
                let timeout = remaining.as_millis().min(100) as i32;
                match poll(&mut pipes, timeout) {
                    Ok(_) | Err(nix::errno::Errno::EINTR) => {}
                    Err(error) => return Err(io::Error::from_raw_os_error(error as i32)),
                }
            }
        }
    }

    impl Drop for Worker {
        fn drop(&mut self) {
            // Child::drop alone neither terminates nor reaps a process.
            // These are harmless if try_wait already reaped this child.
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    fs::write(&path, "1\nExisting/SM\n").unwrap();
    let lock_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(dir.0.join("lingo.dic.cargo-spellcheck.lock"))
        .unwrap();
    lock_file.file().lock().unwrap();
    let executable = std::env::current_exe().unwrap();
    let mut children = (0..4)
        .map(|index| {
            Worker::new(
                Command::new(&executable)
                    // Exact selection of a non-spawning helper prevents recursion.
                    .args([
                        "--exact",
                        "action::learning::dictionary::tests::dictionary_persistence_subprocess",
                        "--ignored",
                        "--nocapture",
                        "--test-threads=1",
                    ])
                    .env("CARGO_SPELLCHECK_DICTIONARY_PROCESS_FIXTURE", &dir.0)
                    .env(
                        "CARGO_SPELLCHECK_DICTIONARY_PROCESS_INDEX",
                        index.to_string(),
                    )
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap(),
            )
        })
        .collect::<Vec<_>>();

    let readiness_deadline = Instant::now() + Duration::from_secs(10);
    for worker in &mut children {
        worker.wait_for(readiness_deadline, true).unwrap();
    }
    for worker in &mut children {
        worker.child.stdin.take().unwrap().write_all(&[1]).unwrap();
    }
    // With the parent still holding the sidecar, completion must time out.
    // An already-expired deadline exercises the timeout path without sleeps.
    assert_eq!(
        children[0]
            .wait_for(Instant::now(), false)
            .unwrap_err()
            .kind(),
        io::ErrorKind::TimedOut
    );
    drop(lock_file);
    let completion_deadline = Instant::now() + Duration::from_secs(10);
    for worker in &mut children {
        worker.wait_for(completion_deadline, false).unwrap();
    }
    drop(children);

    let contents = fs::read_to_string(&path).unwrap();
    let mut lines = contents.lines();
    let mut expected = words(&["Existing/SM", "Shared"]);
    for index in 0..4 {
        expected.extend((0..8).map(|word| format!("Word{}-{}", index, word)));
    }
    assert_eq!(lines.next(), Some("34"));
    let entries = lines.collect::<Vec<_>>();
    assert_eq!(entries.len(), expected.len());
    assert_eq!(
        entries
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>(),
        expected
    );
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
}

#[test]
fn concurrent_sessions_merge_under_the_stable_lock() {
    use std::sync::{Arc, Barrier};
    let dir = TempDir::new();
    let path = dir.0.join("lingo.dic");
    fs::write(&path, "0\n").unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let handles = (0..8)
        .map(|index| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let accepted = words(&[&format!("Word{}", index)]);
                barrier.wait();
                persist_words(&path, &accepted).unwrap();
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap();
    }
    let contents = fs::read_to_string(&path).unwrap();
    let mut lines = contents.lines();
    assert_eq!(lines.next(), Some("8"));
    assert_eq!(
        lines.map(str::to_owned).collect::<BTreeSet<_>>(),
        (0..8).map(|index| format!("Word{}", index)).collect()
    );
}

mod default_dictionary {
    use super::*;

    fn flagged(word: &str) -> DictionaryEntry {
        DictionaryEntry::with_test_flags(word, "S").unwrap()
    }

    #[test]
    fn construction_reads_and_empty_batches_create_nothing() {
        let dir = TempDir::new();
        for path in [
            dir.0.join("missing.dic"),
            dir.0.join("missing/parents/lingo.dic"),
        ] {
            let file = DictionaryFile::new_default(path);
            let store: &dyn DictionaryStore = &file;
            store.validate_word("Word").unwrap();
            assert!(store.read().unwrap().is_empty());
            assert!(store.read_entries().unwrap().is_empty());
            store.update(&BTreeSet::new()).unwrap();
            store.update_entries(&BTreeSet::new()).unwrap();
            drop(file);
            assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
        }
        // Even an unusable parent must not be inspected for an empty batch.
        let parent = dir.0.join("not-a-directory");
        fs::write(&parent, "untouched").unwrap();
        let file = DictionaryFile::new_default(parent.join("lingo.dic"));
        file.update(&BTreeSet::new()).unwrap();
        file.update_entries(&BTreeSet::new()).unwrap();
        assert_eq!(fs::read_to_string(&parent).unwrap(), "untouched");
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn plain_commits_create_parents_and_a_reloadable_dictionary() {
        for use_entries in [false, true] {
            let dir = TempDir::new();
            let path = dir.0.join("missing/parents/lingo.dic");
            let file = DictionaryFile::new_default(path.clone());
            let accepted = words(&["Word", "word", "Éclair", "東京"]);
            if use_entries {
                let entries = accepted
                    .iter()
                    .cloned()
                    .map(DictionaryEntry::plain)
                    .collect::<Result<BTreeSet<_>>>()
                    .unwrap();
                file.update_entries(&entries).unwrap();
            } else {
                file.update(&accepted).unwrap();
            }
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                "4\nWord\nword\nÉclair\n東京\n"
            );
            assert_eq!(file.read().unwrap(), accepted);
            assert_eq!(file.read_entries().unwrap(), accepted);
            let reopened = DictionaryFile::new(path.clone());
            assert_eq!(reopened.read().unwrap(), accepted);
            assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 2);
        }
    }

    #[test]
    fn flagged_commit_creates_and_then_merges_through_the_same_store() {
        let dir = TempDir::new();
        let path = dir.0.join("missing/parents/lingo.dic");
        let file = DictionaryFile::new_default(path.clone());
        let entries = BTreeSet::from([flagged("Word"), DictionaryEntry::plain("Éclair").unwrap()]);
        file.update_entries(&entries).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "2\nWord/S\nÉclair\n");
        assert_eq!(file.read_entries().unwrap(), words(&["Word/S", "Éclair"]));
        assert_eq!(file.read().unwrap(), words(&["Word", "Éclair"]));
        file.update(&words(&["Word"])).unwrap();
        file.update_entries(&entries).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "3\nWord/S\nÉclair\nWord\n"
        );
        assert_eq!(
            file.read_entries().unwrap(),
            words(&["Word", "Word/S", "Éclair"])
        );
        let reopened = DictionaryFile::new(path.clone());
        assert_eq!(
            reopened.read_entries().unwrap(),
            file.read_entries().unwrap()
        );
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 2);
    }

    #[test]
    fn strict_stores_and_public_persistence_still_require_an_existing_dictionary() {
        let dir = TempDir::new();
        for path in [
            dir.0.join("missing.dic"),
            dir.0.join("missing/parents/lingo.dic"),
        ] {
            let file = DictionaryFile::new(path.clone());
            let accepted = words(&["Word"]);
            let entries = BTreeSet::from([flagged("Word")]);
            assert!(file.read().is_err());
            assert!(file.read_entries().is_err());
            assert!(file.update(&accepted).is_err());
            assert!(file.update_entries(&entries).is_err());
            assert!(persist_words(&path, &accepted).is_err());
            assert!(persist_entries(&path, &entries).is_err());
            assert!(!path.exists());
        }
        assert!(!dir.0.join("missing").exists());
        // The strict existing-parent path retains its established sidecar behavior.
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn invalid_batches_and_escaped_stems_never_create_parents_or_locks() {
        let dir = TempDir::new();
        for existing in [false, true] {
            let path = if existing {
                dir.0.join("lingo.dic")
            } else {
                dir.0.join("missing/parents/lingo.dic")
            };
            let original = "1\nExisting/S\n";
            if existing {
                fs::write(&path, original).unwrap();
            }
            let file = DictionaryFile::new_default(path.clone());
            for invalid in [
                "",
                "123",
                "two words",
                "word/S",
                "word\\/S",
                "word\\S",
                "word\tpo:noun",
                "word\nInjected",
                "word\0",
                "#comment",
                "\u{feff}word",
            ] {
                assert!(file.validate_word(invalid).is_err());
                assert!(file.update(&words(&["Accepted", invalid])).is_err());
                assert!(DictionaryEntry::plain(invalid).is_err());
                assert!(DictionaryEntry::with_test_flags(invalid, "S").is_err());
                assert!(!dir.0.join("missing").exists());
                assert_eq!(fs::read_dir(&dir.0).unwrap().count(), usize::from(existing));
                if existing {
                    assert_eq!(fs::read_to_string(&path).unwrap(), original);
                }
            }
        }
    }

    #[test]
    fn existing_malformed_defaults_are_never_initialized_or_replaced() {
        let dir = TempDir::new();
        let path = dir.0.join("lingo.dic");
        let file = DictionaryFile::new_default(path.clone());
        for original in [
            b"".as_ref(),
            b"Word\n",
            b"1\n/SM\n",
            b"1\n\n",
            b"1\nWo\0rd\n",
            b"1\n\xff\n",
        ] {
            fs::write(&path, original).unwrap();
            assert!(file.read().is_err());
            assert!(file.read_entries().is_err());
            assert!(file.update(&words(&["Word"])).is_err());
            assert!(file
                .update_entries(&BTreeSet::from([flagged("Word")]))
                .is_err());
            assert_eq!(fs::read(&path).unwrap(), original);
            assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
        }
    }

    #[cfg(unix)]
    #[test]
    fn new_defaults_are_private_and_existing_permissions_are_preserved() {
        use std::os::unix::fs::PermissionsExt;

        for use_entries in [false, true] {
            let dir = TempDir::new();
            let path = dir.0.join("missing/lingo.dic");
            let file = DictionaryFile::new_default(path.clone());
            if use_entries {
                file.update_entries(&BTreeSet::from([flagged("First")]))
                    .unwrap();
            } else {
                file.update(&words(&["First"])).unwrap();
            }
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
            if use_entries {
                file.update_entries(&BTreeSet::from([flagged("Second")]))
                    .unwrap();
            } else {
                file.update(&words(&["Second"])).unwrap();
            }
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o640
            );
        }
    }

    #[test]
    fn directory_default_is_not_replaced() {
        let dir = TempDir::new();
        let path = dir.0.join("lingo.dic");
        fs::create_dir(&path).unwrap();
        let file = DictionaryFile::new_default(path.clone());
        assert!(file.read().is_err());
        assert!(file.read_entries().is_err());
        assert!(file.update(&words(&["Word"])).is_err());
        assert!(file
            .update_entries(&BTreeSet::from([flagged("Word")]))
            .is_err());
        assert!(path.is_dir());
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn dangling_and_live_symlink_defaults_are_not_replaced() {
        use std::os::unix::fs::symlink;

        for target_exists in [false, true] {
            let dir = TempDir::new();
            let path = dir.0.join("lingo.dic");
            let target = dir.0.join("target.dic");
            if target_exists {
                fs::write(&target, "1\nExisting/S\n").unwrap();
            }
            symlink(&target, &path).unwrap();
            let file = DictionaryFile::new_default(path.clone());
            assert!(file.read().is_err());
            assert!(file.read_entries().is_err());
            assert!(file.update(&words(&["Word"])).is_err());
            assert!(file
                .update_entries(&BTreeSet::from([flagged("Word")]))
                .is_err());
            assert!(fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(fs::read_link(&path).unwrap(), target);
            if target_exists {
                assert_eq!(fs::read_to_string(&target).unwrap(), "1\nExisting/S\n");
            } else {
                assert!(!target.exists());
            }
            assert_eq!(
                fs::read_dir(&dir.0).unwrap().count(),
                2 + usize::from(target_exists)
            );
        }
    }

    #[test]
    fn waiting_commit_does_not_initialize_before_locking_and_rereads_after_locking() {
        use std::sync::mpsc;
        use std::time::Duration;

        let dir = TempDir::new();
        let path = dir.0.join("lingo.dic");
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(dir.0.join("lingo.dic.cargo-spellcheck.lock"))
            .unwrap();
        lock.file().lock().unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let file = DictionaryFile::new_default(path.clone());
        let handle = std::thread::spawn(move || {
            ready_tx.send(()).unwrap();
            file.update_entries(&BTreeSet::from([flagged("Word")]))
                .unwrap();
            done_tx.send(()).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let waiting = matches!(
            done_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        let absent_while_locked = !path.exists();
        fs::write(&path, "1\nOther/S\n").unwrap();
        drop(lock);
        if waiting {
            done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        handle.join().unwrap();
        assert!(waiting, "commit did not wait for the stable sidecar lock");
        assert!(
            absent_while_locked,
            "dictionary was initialized before locking"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "2\nOther/S\nWord/S\n");
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
    }

    #[test]
    fn concurrent_initial_plain_and_flagged_commits_merge_under_the_stable_lock() {
        use std::sync::{Arc, Barrier};

        let dir = TempDir::new();
        let path = dir.0.join("missing/parents/lingo.dic");
        let barrier = Arc::new(Barrier::new(8));
        let handles = (0..8)
            .map(|index| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let file = DictionaryFile::new_default(path);
                    let word = format!("Word{}", index / 2);
                    barrier.wait();
                    if index % 2 == 0 {
                        file.update(&words(&["Shared", &word])).unwrap();
                    } else {
                        file.update_entries(&BTreeSet::from([flagged("Shared"), flagged(&word)]))
                            .unwrap();
                    }
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }
        let mut expected = words(&["Shared", "Shared/S"]);
        for index in 0..4 {
            expected.insert(format!("Word{index}"));
            expected.insert(format!("Word{index}/S"));
        }
        let file = DictionaryFile::new(path.clone());
        assert_eq!(file.read_entries().unwrap(), expected);
        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents.lines().next(), Some("10"));
        assert_eq!(contents.lines().skip(1).count(), 10);
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 2);
    }
}

mod confirmed_entries {
    use super::*;

    // Storage fixtures do not imply engine confirmation or affix validation.
    fn flagged_fixture(word: &str, flags: &str) -> DictionaryEntry {
        DictionaryEntry::with_test_flags(word, flags).unwrap()
    }

    #[test]
    fn roundtrip_deduplicates_morphology_but_keeps_distinct_flagged_entries() {
        let dir = TempDir::new();
        let path = dir.0.join("lingo.dic");
        let file = DictionaryFile::new(path.clone());
        let store: &dyn DictionaryStore = &file;
        let accepted = BTreeSet::from([
            flagged_fixture("Existing", "S"),
            DictionaryEntry::plain("Other").unwrap(),
            DictionaryEntry::plain("Word").unwrap(),
            flagged_fixture("Word", "D"),
            flagged_fixture("Word", "S"),
            flagged_fixture("word", "S"),
            flagged_fixture("Éclair", "S"),
        ]);
        for newline in ["\n", "\r\n"] {
            let original = concat!(
                "99\n",
                "Existing/S\tpo:noun st:existing/metadata\n",
                "Other po:noun\n",
                "Untouched/DS\n",
            );
            fs::write(&path, original.replace('\n', newline)).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
            }

            store.update_entries(&accepted).unwrap();

            let expected = concat!(
                "8\n",
                "Existing/S\tpo:noun st:existing/metadata\n",
                "Other po:noun\n",
                "Untouched/DS\n",
                "Word\n",
                "Word/D\n",
                "Word/S\n",
                "word/S\n",
                "Éclair/S\n",
            )
            .replace('\n', newline);
            assert_eq!(fs::read_to_string(&path).unwrap(), expected);
            let tokens = words(&[
                "Existing/S",
                "Other",
                "Untouched/DS",
                "Word",
                "Word/D",
                "Word/S",
                "word/S",
                "Éclair/S",
            ]);
            assert_eq!(store.read_entries().unwrap(), tokens);
            assert_eq!(
                store.read().unwrap(),
                words(&["Existing", "Other", "Untouched", "Word", "word", "Éclair"])
            );
            assert!(accepted
                .iter()
                .all(|entry| tokens.contains(&entry.to_line())));
            store.update_entries(&accepted).unwrap();
            persist_entries(&path, &accepted).unwrap();
            assert_eq!(fs::read_to_string(&path).unwrap(), expected);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                    0o640
                );
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn duplicate_flagged_tokens_with_morphology_do_not_replace_dictionary() {
        use std::os::unix::fs::MetadataExt;

        let dir = TempDir::new();
        let path = dir.0.join("lingo.dic");
        let link = dir.0.join("original.dic");
        let original = "2\r\nWord/S\tpo:noun\r\nWord po:verb\r\n";
        fs::write(&path, original).unwrap();
        fs::hard_link(&path, &link).unwrap();
        let before = fs::metadata(&link).unwrap();
        let accepted = BTreeSet::from([
            flagged_fixture("Word", "S"),
            DictionaryEntry::plain("Word").unwrap(),
        ]);

        persist_entries(&path, &accepted).unwrap();

        let after = fs::metadata(&path).unwrap();
        assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
        assert_eq!(after.modified().unwrap(), before.modified().unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert_eq!(fs::read_to_string(&link).unwrap(), original);
    }

    #[test]
    fn concurrent_plain_and_flagged_sessions_merge_under_the_same_lock() {
        use std::sync::{Arc, Barrier};

        let dir = TempDir::new();
        let path = dir.0.join("lingo.dic");
        fs::write(&path, "1\r\nExisting/D\tpo:noun\r\n").unwrap();
        let barrier = Arc::new(Barrier::new(8));
        let handles = (0..8)
            .map(|index| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                let word = format!("Word{}", index / 2);
                let plain = words(&["Shared", &word]);
                let flagged =
                    BTreeSet::from([flagged_fixture("Shared", "S"), flagged_fixture(&word, "S")]);
                std::thread::spawn(move || {
                    barrier.wait();
                    if index % 2 == 0 {
                        persist_words(&path, &plain).unwrap();
                    } else {
                        persist_entries(&path, &flagged).unwrap();
                    }
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.starts_with("11\r\nExisting/D\tpo:noun\r\n"));
        assert_eq!(contents.lines().skip(1).count(), 11);
        assert_eq!(contents.matches("\r\n").count(), 12);
        let mut expected = words(&["Existing/D", "Shared", "Shared/S"]);
        for index in 0..4 {
            expected.insert(format!("Word{index}"));
            expected.insert(format!("Word{index}/S"));
        }
        let file = DictionaryFile::new(path);
        let store: &dyn DictionaryStore = &file;
        assert_eq!(store.read_entries().unwrap(), expected);
    }
}
