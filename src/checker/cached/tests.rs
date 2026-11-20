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
