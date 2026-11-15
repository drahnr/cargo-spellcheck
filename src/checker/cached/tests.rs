use super::*;

#[test]
fn cache_operations_release_locks() {
    let cache_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let mut writer = Cached::<String>::new("test", &cache_dir).unwrap();
    writer.update(&"cached value".to_owned()).unwrap();

    let mut reader = Cached::<String>::new("test", &cache_dir).unwrap();
    reader.cache_file.file().try_lock().unwrap();
    reader.cache_file.file().unlock().unwrap();
    assert_eq!(reader.fetch().unwrap(), Some("cached value".to_owned()));
    writer.cache_file.file().try_lock().unwrap();
    writer.cache_file.file().unlock().unwrap();

    // A second shared reader must not require exclusive access.
    writer.cache_file.file().lock_shared().unwrap();
    let mut another_reader = Cached::<String>::new("test", &cache_dir).unwrap();
    assert_eq!(
        another_reader.fetch().unwrap(),
        Some("cached value".to_owned())
    );
    writer.cache_file.file().unlock().unwrap();

    drop((writer, reader, another_reader));
    fs_err::remove_dir_all(cache_dir).unwrap();
}

#[test]
fn failed_cache_update_releases_lock() {
    let cache_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let mut cache = Cached::<String>::new("test", &cache_dir).unwrap();
    let path = fs_err::read_dir(&cache_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    cache.cache_file = fs_err::File::open(&path).unwrap();
    assert!(cache.update(&"cannot write".to_owned()).is_err());

    let other_file = fs_err::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    other_file.file().try_lock().unwrap();
    drop((cache, other_file));
    fs_err::remove_dir_all(cache_dir).unwrap();
}

#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
mod builtin {
    use super::*;
    use std::io::Write;

    #[test]
    fn cache_builtin_ignores_legacy_affix_file() {
        let cache_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let legacy = cache_dir.join(format!(
            "cargo-spellcheck/{}/en_US.aff",
            env!("CARGO_PKG_VERSION")
        ));
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, b"SET UTF8\n").unwrap();

        let path = cache_builtin_inner(&cache_dir, "aff", BUILTIN_HUNSPELL_AFF).unwrap();
        assert_ne!(path, legacy);
        assert_eq!(fs::read(&path).unwrap(), BUILTIN_HUNSPELL_AFF);
        assert_eq!(
            cache_builtin_inner(&cache_dir, "aff", BUILTIN_HUNSPELL_AFF).unwrap(),
            path
        );
        // Older processes may still use the legacy path; do not overwrite it.
        assert_eq!(fs::read(&legacy).unwrap(), b"SET UTF8\n");
        fs::remove_dir_all(cache_dir).unwrap();
    }

    #[test]
    fn cache_builtin_writes_and_reuses_existing_file() {
        let cache_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let data = b"cached dictionary";
        let path = cache_builtin_inner(&cache_dir, "dic", data).unwrap();
        assert_eq!(fs::read(&path).unwrap(), data);

        let f = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        // The writer's lock must have been released when caching returned.
        f.file().try_lock().unwrap();
        drop(f);

        let f = fs::File::open(&path).unwrap();
        f.file().lock_shared().unwrap();
        // Existing cache files can be reused alongside other shared readers.
        assert_eq!(cache_builtin_inner(&cache_dir, "dic", data).unwrap(), path);
        assert_eq!(fs::read(&path).unwrap(), data);
        drop(f);

        let replacement = cache_builtin_inner(&cache_dir, "dic", b"replacement").unwrap();
        assert_ne!(replacement, path);
        assert_eq!(fs::read(replacement).unwrap(), b"replacement");
        assert_eq!(fs::read(&path).unwrap(), data);

        let f = fs::OpenOptions::new().write(true).open(&path).unwrap();
        // The cache reader must not retain its shared lock after returning.
        f.file().try_lock().unwrap();
        drop(f);
        fs::remove_dir_all(cache_dir).unwrap();
    }

    #[test]
    fn cache_builtin_waits_for_writer() {
        use std::sync::mpsc;
        use std::time::Duration;
        let cache_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let path = cache_builtin_inner(&cache_dir, "dic", b"complete dictionary").unwrap();
        let mut writer = fs::OpenOptions::new().write(true).open(&path).unwrap();
        writer.file().lock().unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let reader_cache_dir = cache_dir.clone();
        let reader = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let path =
                cache_builtin_inner(reader_cache_dir, "dic", b"complete dictionary").unwrap();
            done_tx.send(path).unwrap();
        });

        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            done_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        writer.write_all(b"complete dictionary").unwrap();
        drop(writer);

        assert_eq!(done_rx.recv_timeout(Duration::from_secs(5)).unwrap(), path);
        reader.join().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"complete dictionary");
        fs::remove_dir_all(cache_dir).unwrap();
    }
}
