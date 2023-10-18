use crate::errors::*;

use hex::ToHex;
use serde::de::DeserializeOwned;
use serde::ser::Serialize;
use sha2::Digest;
use std::io::Seek;
use std::path::Path;
use std::time::{Duration, Instant};

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

pub struct Cached<T> {
    cache_file: fd_lock::RwLock<fs_err::File>,
    // What to cache.
    what: String,
    _phantom: std::marker::PhantomData<T>,
}

impl<'a, T> Cached<T>
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
            cache_file: fd_lock::RwLock::new(cache_file),
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
                    fetch: Some(elapsed.clone()),
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
        let guard = self.cache_file.read()?;
        let buf = std::io::BufReader::new(guard.file());
        // let buf = xz2::bufread::XzDecoder::new(buf);
        match bincode::deserialize_from(buf) {
            Ok(CacheEntry { what, val }) => {
                if &what == &self.what {
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
        let mut write_guard = self.cache_file.write()?;

        let entry = CacheEntry {
            what: self.what.clone(),
            val,
        };
        let encoded: Vec<u8> = bincode::serialize(&entry).unwrap();
        let mut encoded = &encoded[..];
        // let mut compressed = xz2::bufread::XzEncoder::new(&mut encoded, 6);

        // effectively truncate, but without losing the lock
        let file = write_guard.file_mut();
        file.rewind()?;
        std::io::copy(&mut encoded, file)?;
        let loco = file.stream_position()?;
        file.set_len(loco)?;
        Ok(())
    }
}
