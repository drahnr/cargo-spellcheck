use crate::errors::*;

#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
use fs_err as fs;
use hex::ToHex;
use serde::de::DeserializeOwned;
use serde::ser::Serialize;
use sha2::Digest;
use std::io::Seek;
#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
use std::io::Write;
use std::path::Path;
#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CacheEntry<T> {
    what: String,
    val: T,
}

pub struct CachedValue<T> {
    /// Time it took to..
    /// load the value from disk if it was there.
    pub fetch: Option<Duration>,
    /// Updating the disk cache
    pub update: Option<Duration>,
    /// Create a new one if needed
    pub creation: Option<Duration>,
    /// The accumulated duration,
    pub total: Duration,
    /// The actual value.
    pub value: T,
}

// Cached keeps its file open between operations, so release each lock even on errors.
struct FileLockGuard<'a>(&'a fs_err::File);

impl Drop for FileLockGuard<'_> {
    fn drop(&mut self) {
        if let Err(err) = self.0.file().unlock() {
            log::warn!("Failed to unlock cache file: {err:?}");
        }
    }
}

pub struct Cached<T> {
    cache_file: fs_err::File,
    // What to cache.
    what: String,
    _phantom: std::marker::PhantomData<T>,
}

impl<T> Cached<T>
where
    T: Serialize + DeserializeOwned,
{
    /// Create a new `Cached` instance, to create a expanded version of something that's identified by `what`.
    pub fn new(what: impl AsRef<str>, cache_dir: impl AsRef<Path>) -> Result<Self> {
        let what = what.as_ref();
        let what_digest = sha2::Sha256::digest(what.as_bytes());
        let cache_dir = cache_dir.as_ref();
        fs_err::create_dir_all(cache_dir)?;
        let cache_file = cache_dir.join(what_digest.as_slice().encode_hex::<String>());
        let cache_file = fs_err::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(cache_file)?;
        Ok(Self {
            cache_file,
            what: what.to_owned(),
            _phantom: std::marker::PhantomData,
        })
    }

    pub fn fetch_or_update(
        &mut self,
        create: impl FnOnce(&str) -> Result<T>,
    ) -> Result<CachedValue<T>> {
        let total_start = Instant::now();
        match self.fetch() {
            Ok(Some(value)) => {
                let elapsed = total_start.elapsed();
                Ok(CachedValue {
                    value,
                    fetch: Some(elapsed),
                    update: None,
                    creation: None,
                    total: elapsed,
                })
            }
            Ok(None) => {
                let fetch = Some(total_start.elapsed());

                let creation_start = Instant::now();
                let value = create(self.what.as_str())?;
                let creation = Some(creation_start.elapsed());

                let update_start = Instant::now();
                if let Err(err) = self.update(&value) {
                    log::warn!("Failed to write value to cached: {err:?}");
                }
                let update = Some(update_start.elapsed());
                let total = total_start.elapsed();
                Ok(CachedValue {
                    value,
                    fetch,
                    update,
                    creation,
                    total,
                })
            }
            Err(err) => {
                log::warn!("Overriding existing value that failed to load: {err:?}");

                let fetch = Some(total_start.elapsed());

                let creation_start = Instant::now();
                let value = create(self.what.as_str())?;
                let creation = Some(creation_start.elapsed());

                let update_start = Instant::now();
                if let Err(err) = self.update(&value) {
                    log::warn!("Failed to update cached: {err:?}");
                }
                let update = Some(update_start.elapsed());
                let total = total_start.elapsed();
                Ok(CachedValue {
                    value,
                    fetch,
                    update,
                    creation,
                    total,
                })
            }
        }
    }
    pub fn fetch(&mut self) -> Result<Option<T>> {
        self.cache_file.file().lock_shared()?;
        let guard = FileLockGuard(&self.cache_file);
        let buf = std::io::BufReader::new(guard.0);
        // let buf = xz2::bufread::XzDecoder::new(buf);
        match bincode::deserialize_from(buf) {
            Ok(CacheEntry { what, val }) => {
                if what == self.what {
                    log::debug!("Cached value with matching what \"{what}\"");
                    Ok(Some(val))
                } else {
                    log::warn!(
                        "Cached value what \"{}\" does not match expect what \"{}\", removing",
                        what,
                        self.what
                    );
                    Ok(None)
                }
            }
            Err(e) => {
                log::warn!("Failed to load cached value: {e:?}");
                Ok(None)
            }
        }
    }

    pub fn update(&mut self, val: &T) -> Result<()> {
        self.cache_file.file().lock()?;
        let guard = FileLockGuard(&self.cache_file);

        let entry = CacheEntry {
            what: self.what.clone(),
            val,
        };
        let encoded: Vec<u8> = bincode::serialize(&entry).unwrap();
        let mut encoded = &encoded[..];
        // let mut compressed = xz2::bufread::XzEncoder::new(&mut encoded, 6);

        // effectively truncate, but without losing the lock
        let mut file = guard.0;
        file.rewind()?;
        std::io::copy(&mut encoded, &mut file)?;
        let loco = file.stream_position()?;
        file.set_len(loco)?;
        Ok(())
    }
}

#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
pub(super) static BUILTIN_HUNSPELL_AFF: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/hunspell-data/en_US.aff"
));

#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
pub(super) static BUILTIN_HUNSPELL_DIC: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/hunspell-data/en_US.dic"
));

// XXX hunspell does not provide an API for using in-memory dictionary or
// XXX affix files
// XXX https://github.com/hunspell/hunspell/issues/721
#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
pub(super) fn cache_builtin_inner(
    cache_dir: impl AsRef<Path>,
    extension: &'static str,
    data: &[u8],
) -> Result<PathBuf> {
    // Embedded data can change without a package version bump. Never reuse
    // stale affix headers (notably SET UTF8 instead of SET UTF-8).
    let digest = hex::encode(sha2::Sha256::digest(data));
    let path = cache_dir.as_ref().join(format!(
        "cargo-spellcheck/{}/{digest}/en_US.{extension}",
        env!("CARGO_PKG_VERSION"),
    ));
    fs::create_dir_all(path.parent().unwrap())?;
    // check if file exists
    if let Ok(f) = fs::File::open(&path) {
        // in case somebody else is currently writing to it
        // wait for that to complete
        f.file().lock_shared()?;
        return Ok(path);
    }
    let mut f = fs::OpenOptions::new()
        .truncate(false)
        .create(true)
        // Shared locking requires read access on Windows.
        .read(true)
        .write(true)
        .open(&path)?;
    // if there are multiple instances, allow the first to write it all
    match f.file().try_lock() {
        Ok(()) => {
            // Do not truncate another writer's data before acquiring the lock.
            f.set_len(0)?;
            f.write_all(data)?;
            return Ok(path);
        }
        Err(std::fs::TryLockError::WouldBlock) => {}
        Err(std::fs::TryLockError::Error(err)) => return Err(err.into()),
    }

    // .. but block execution until the first completed so
    // there are no cases of partial data
    f.file().lock_shared()?;

    Ok(path)
}

#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
pub(super) fn cache_builtin() -> Result<(PathBuf, PathBuf)> {
    log::info!("Using builtin en_US hunspell dictionary and affix files");
    let base = directories::BaseDirs::new().expect("env HOME must be set");

    let cache_dir = base.cache_dir();
    let path_aff = cache_builtin_inner(cache_dir, "aff", BUILTIN_HUNSPELL_AFF)?;
    let path_dic = cache_builtin_inner(cache_dir, "dic", BUILTIN_HUNSPELL_DIC)?;
    Ok((path_dic, path_aff))
}
