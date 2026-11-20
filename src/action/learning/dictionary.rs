//! Backend-independent validation and persistence in Hunspell dictionary format.

use super::entry::DictionaryEntry;
use super::DictionaryStore;
use crate::errors::*;
use fs_err as fs;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

pub(super) struct DictionaryFile {
    path: PathBuf,
    allow_create: bool,
}

impl DictionaryFile {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            allow_create: false,
        }
    }

    /// A missing learning dictionary stays in memory until a nonempty commit.
    pub(super) fn new_default(path: PathBuf) -> Self {
        Self {
            path,
            allow_create: true,
        }
    }
}

impl DictionaryStore for DictionaryFile {
    fn validate_word(&self, word: &str) -> Result<()> {
        validate_word(word)
    }

    fn update_entries(&self, entries: &BTreeSet<DictionaryEntry>) -> Result<()> {
        persist_entries(&self.path, entries, self.allow_create).wrap_err_with(|| {
            format!(
                "Could not persist accepted entries to {}",
                self.path.display()
            )
        })
    }

    fn read_entries(&self) -> Result<BTreeSet<String>> {
        let (contents, _) = read_dictionary(&self.path, self.allow_create)?;
        Ok(parse_dictionary(&contents)?
            .entry_tokens
            .into_iter()
            .map(str::to_owned)
            .collect())
    }
}

/// Accept a single literal word, never flags, morphology, or dictionary syntax.
pub(super) fn validate_word(word: &str) -> Result<()> {
    super::entry::validate_word(word)
}

fn read_dictionary(
    path: &Path,
    allow_missing: bool,
) -> Result<(String, Option<std::fs::Permissions>)> {
    let metadata = if allow_missing {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                if !metadata.is_file() {
                    bail!(
                        "Learning dictionary is not a regular file: {}",
                        path.display()
                    );
                }
                metadata
            }
            // Unlike exists() or metadata(), this does not mistake a dangling
            // symlink for an absent dictionary that may safely be replaced.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(("0\n".to_owned(), None));
            }
            Err(error) => return Err(error).wrap_err("Could not read a UTF-8 dictionary"),
        }
    } else {
        fs::metadata(path).wrap_err("Could not read a UTF-8 dictionary")?
    };
    let contents = fs::read_to_string(path).wrap_err("Could not read a UTF-8 dictionary")?;
    Ok((contents, Some(metadata.permissions())))
}

struct ParsedDictionary<'a> {
    entries: Vec<&'a str>,
    entry_tokens: BTreeSet<&'a str>,
    plain_stems: BTreeSet<&'a str>,
}

fn parse_dictionary(contents: &str) -> Result<ParsedDictionary<'_>> {
    let mut lines = contents.lines();
    let header = lines
        .next()
        .ok_or_else(|| eyre!("Dictionary is empty; expected a numeric first line"))?;
    if header.is_empty()
        || !header.bytes().all(|ch| ch.is_ascii_digit())
        || header.parse::<usize>().is_err()
    {
        bail!("Malformed dictionary: expected a numeric entry count on the first line");
    }
    let mut entries = Vec::new();
    let mut entry_tokens = BTreeSet::new();
    let mut plain_stems = BTreeSet::new();
    for (index, entry) in lines.enumerate() {
        let stem = entry
            .split(|ch: char| ch == '/' || ch.is_whitespace())
            .next()
            .unwrap_or("");
        if stem.is_empty() || entry.chars().any(|ch| ch.is_control() && ch != '\t') {
            bail!("Malformed dictionary entry on line {}", index + 2);
        }
        entry_tokens.insert(entry.split(char::is_whitespace).next().unwrap_or(""));
        if !entry[stem.len()..].starts_with('/') {
            plain_stems.insert(stem);
        }
        entries.push(entry);
    }
    Ok(ParsedDictionary {
        entries,
        entry_tokens,
        plain_stems,
    })
}

/// Merge confirmed entries, validating the whole batch before any filesystem writes.
fn persist_entries(
    path: &Path,
    accepted: &BTreeSet<DictionaryEntry>,
    allow_create: bool,
) -> Result<()> {
    if accepted.is_empty() {
        return Ok(());
    }
    for entry in accepted {
        validate_word(entry.word())?;
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if allow_create {
        fs::create_dir_all(parent).wrap_err("Could not create learning dictionary parents")?;
    }
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".cargo-spellcheck.lock");
    let lock_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(Path::new(&lock_path))
        .wrap_err("Could not open the dictionary sidecar lock")?;
    // Lock a stable inode, not the dictionary inode replaced by rename.
    // Never delete this sidecar: another session may already be waiting on it.
    // Dropping this file releases the lock, including on error paths.
    lock_file
        .file()
        .lock()
        .wrap_err("Could not lock the learning dictionary")?;
    // Do not defer shutdown while waiting for another process's file lock.
    let _hat = crate::TinHat::on();

    let (contents, permissions) = read_dictionary(path, allow_create)?;
    let ParsedDictionary {
        entries,
        entry_tokens,
        plain_stems,
    } = parse_dictionary(&contents)?;
    // Flagged entries may forbid standalone use; explicit plain acceptance
    // still needs an unflagged entry, with or without morphology. Confirmed
    // flags instead deduplicate only against the exact existing entry token.
    let additions = accepted
        .iter()
        .filter(|entry| match entry.flags() {
            None => !plain_stems.contains(entry.word()),
            Some(_) => !entry_tokens.contains(entry.to_line().as_str()),
        })
        .collect::<Vec<_>>();
    // Recount entries rather than trusting a stale count in the old header.
    let count = entries.len() + additions.len();
    let newline = if contents.split('\n').next().unwrap_or("").ends_with('\r') {
        "\r\n"
    } else {
        "\n"
    };
    let mut merged = format!("{}{}", count, newline);
    for entry in entries {
        merged.push_str(entry);
        merged.push_str(newline);
    }
    for entry in additions {
        merged.push_str(&entry.to_line());
        merged.push_str(newline);
    }
    if merged == contents {
        return Ok(());
    }

    let name = path
        .file_name()
        .ok_or_else(|| eyre!("Dictionary path has no file name"))?;
    let mut temporary_name = OsString::from(".");
    temporary_name.push(name);
    temporary_name.push(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let temporary = parent.join(temporary_name);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use fs::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .wrap_err("Could not create a temporary dictionary alongside the destination")?;
    let result = (|| -> Result<()> {
        // Apply destination permissions before writing any dictionary content.
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        file.write_all(merged.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        // In particular, Windows cannot rename over an open file.
        drop(file);
        fs::rename(&temporary, path).wrap_err("Could not replace the learning dictionary")?;
        Ok(())
    })();
    if result.is_err() {
        if let Err(error) = fs::remove_file(&temporary) {
            log::warn!(
                "Could not remove temporary dictionary {}: {}",
                temporary.display(),
                error
            );
        }
    }
    result
}

#[cfg(test)]
mod tests;
