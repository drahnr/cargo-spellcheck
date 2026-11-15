//! Backend-independent validation and opt-in persistence in Hunspell dictionary format.

use super::DictionaryStore;
use crate::errors::*;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub(super) struct DictionaryFile {
    path: PathBuf,
}

impl DictionaryFile {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl DictionaryStore for DictionaryFile {
    fn validate_word(&self, word: &str) -> Result<()> {
        validate_word(word)
    }

    fn read(&self) -> Result<BTreeSet<String>> {
        let contents =
            fs_err::read_to_string(&self.path).wrap_err("Could not read a UTF-8 dictionary")?;
        Ok(parse_dictionary(&contents)?
            .stems
            .into_iter()
            .map(str::to_owned)
            .collect())
    }

    fn update(&self, words: &BTreeSet<String>) -> Result<()> {
        persist_words(&self.path, words)
    }
}

/// Accept a single literal word, never flags, morphology, or dictionary syntax.
pub(super) fn validate_word(word: &str) -> Result<()> {
    if word.is_empty()
        || word.parse::<i64>().is_ok()
        || word.starts_with('#')
        || word.chars().any(|ch| {
            ch.is_whitespace() || ch.is_control() || matches!(ch, '/' | '\\' | '\u{feff}')
        })
    {
        bail!(
            "Cannot accept {:?}: expected one nonnumeric plain Hunspell word without whitespace, control characters, flags, escapes, or dictionary syntax",
            word
        );
    }
    Ok(())
}

/// Merge accepted words into an existing UTF-8 dictionary without changing TOML.
///
/// Validate the whole batch before opening even the sidecar lock file.
pub(super) fn persist_words(path: &Path, words: &BTreeSet<String>) -> Result<()> {
    if words.is_empty() {
        return Ok(());
    }
    for word in words {
        validate_word(word)?;
    }

    persistence::persist(path, words)
        .wrap_err_with(|| format!("Could not persist accepted words to {}", path.display()))
}

struct ParsedDictionary<'a> {
    entries: Vec<&'a str>,
    stems: BTreeSet<&'a str>,
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
    let mut stems = BTreeSet::new();
    let mut plain_stems = BTreeSet::new();
    for (index, entry) in lines.enumerate() {
        let stem = entry
            .split(|ch: char| ch == '/' || ch.is_whitespace())
            .next()
            .unwrap_or("");
        if stem.is_empty() || entry.chars().any(|ch| ch.is_control() && ch != '\t') {
            bail!("Malformed dictionary entry on line {}", index + 2);
        }
        stems.insert(stem);
        if !entry[stem.len()..].starts_with('/') {
            plain_stems.insert(stem);
        }
        entries.push(entry);
    }
    Ok(ParsedDictionary {
        entries,
        stems,
        plain_stems,
    })
}

mod persistence {
    use super::*;
    use fs_err as fs;
    use std::ffi::OsString;
    use std::io::Write;

    pub(super) fn persist(path: &Path, words: &BTreeSet<String>) -> Result<()> {
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

        let contents = fs::read_to_string(path).wrap_err("Could not read a UTF-8 dictionary")?;
        let ParsedDictionary {
            entries,
            plain_stems,
            ..
        } = parse_dictionary(&contents)?;
        // Flagged entries may forbid standalone use; explicit acceptance needs
        // an unflagged entry, with or without morphology, for the same stem.
        let additions = words
            .iter()
            .filter(|word| !plain_stems.contains(word.as_str()))
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
        for word in additions {
            merged.push_str(word);
            merged.push_str(newline);
        }
        if merged == contents {
            return Ok(());
        }

        let permissions = fs::metadata(path)?.permissions();
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let name = path
            .file_name()
            .ok_or_else(|| eyre!("Dictionary path has no file name"))?;
        let mut temporary_name = OsString::from(".");
        temporary_name.push(name);
        temporary_name.push(format!(".{}.tmp", uuid::Uuid::new_v4()));
        let temporary = parent.join(temporary_name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .wrap_err("Could not create a temporary dictionary alongside the destination")?;
        let result = (|| -> Result<()> {
            // Apply destination permissions before writing any dictionary content.
            file.set_permissions(permissions)?;
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
}

#[cfg(test)]
mod tests;
