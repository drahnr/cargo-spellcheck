//! Backend-neutral session vocabulary and its explicit commit boundaries.

use super::{interactive::UserPicked, update::SourceUpdates};
use crate::checker::DictionaryReload;
use crate::errors::*;
use crate::{Config, Suggestion};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

mod affix;
mod dictionary;
mod entry;

pub(super) use affix::{AffixPattern, AffixSuggestion};
pub(super) use entry::DictionaryEntry;

/// A vocabulary store owns its format, validation, and locked read-merge-write.
/// Engines consume its persisted file through their separate reload interface.
pub(super) trait DictionaryStore {
    fn validate_word(&self, word: &str) -> Result<()>;
    /// Exact dictionary tokens, excluding morphology but retaining flags.
    fn read_entries(&self) -> Result<BTreeSet<String>>;
    fn update_entries(&self, entries: &BTreeSet<DictionaryEntry>) -> Result<()>;
}

/// One shared overlay for all spelling backends; engine instances stay immutable
/// during interaction. Dropping this value without committing writes nothing.
#[derive(Default)]
pub(super) struct SessionResolutions {
    accepted_words: BTreeSet<String>,
    pending_entries: BTreeMap<DictionaryEntry, BTreeSet<String>>,
    config: Option<Config>,
    store: Option<Box<dyn DictionaryStore>>,
    learning_path: Option<PathBuf>,
}

impl SessionResolutions {
    pub(super) fn new(config: &Config, fallback_base: &Path) -> Result<Self> {
        let configured = config.learning_dictionary()?;
        let target = match configured {
            Some(path) => Some(path),
            None if config.has_spelling_backends() => {
                Some(Config::default_learning_dictionary(fallback_base)?)
            }
            None => None,
        };
        let mut config = config.clone();
        let mut store = None;
        let mut learning_path = None;
        if let Some(target) = target {
            let (path, missing) = Config::validate_learning_dictionary(&target)?;
            let dictionary = if missing {
                config.omit_missing_learning_dictionary(&path)?;
                eprintln!(
                    "Warning: learning dictionary {} does not exist; a new file will be created when learned entries are saved.",
                    path.display()
                );
                dictionary::DictionaryFile::new_default(path.clone())
            } else {
                dictionary::DictionaryFile::new(path.clone())
            };
            log::info!("Accepted words will be saved to {}", path.display());
            store = Some(Box::new(dictionary) as Box<dyn DictionaryStore>);
            learning_path = Some(path);
        }
        Ok(Self {
            store,
            learning_path,
            config: Some(config),
            ..Self::default()
        })
    }

    fn validate_word(&self, word: &str) -> Result<()> {
        match &self.store {
            Some(store) => store.validate_word(word),
            None => dictionary::validate_word(word),
        }
    }

    pub(super) fn can_accept(&self, suggestion: &Suggestion<'_>) -> bool {
        suggestion
            .checked_word
            .as_deref()
            .is_some_and(|word| self.validate_word(word).is_ok())
    }

    pub(super) fn accepts(&self, suggestion: &Suggestion<'_>) -> bool {
        suggestion
            .checked_word
            .as_ref()
            .is_some_and(|word| self.knows_word(word))
    }

    pub(super) fn knows_word(&self, word: &str) -> bool {
        self.accepted_words.contains(word)
    }

    pub(super) fn accept(&mut self, suggestion: &Suggestion<'_>) -> Result<()> {
        let word = suggestion
            .checked_word
            .as_deref()
            .ok_or_else(|| eyre!("Cannot accept vocabulary without the original checked word"))?;
        self.validate_word(word)?;
        self.remember_entry(
            DictionaryEntry::plain(word)?,
            BTreeSet::from([word.to_owned()]),
        );
        Ok(())
    }

    #[cfg(all(
        test,
        any(feature = "hunspell", feature = "zet", feature = "spellbook")
    ))]
    pub(super) fn is_persistent(&self) -> bool {
        self.store.is_some()
    }

    pub(super) fn learning_dictionary_path(&self) -> Option<&Path> {
        self.learning_path.as_deref()
    }

    pub(super) fn validate_learning_word(&self, word: &str) -> Result<()> {
        self.validate_word(word)
    }

    pub(super) fn entry_suggestions(&self, word: &str) -> Result<Vec<AffixSuggestion>> {
        self.validate_word(word)?;
        let config = self.config.as_ref().ok_or_else(|| {
            eyre!("Affix learning unavailable: no spelling configuration; plain-word acceptance is still available")
        })?;
        affix::suggestions(config, word)
    }

    pub(super) fn pattern_suggestions(&self, word: &str) -> Result<Vec<AffixPattern>> {
        self.validate_word(word)?;
        let config = self.config.as_ref().ok_or_else(|| {
            eyre!("Affix learning unavailable: no spelling configuration; plain-word acceptance is still available")
        })?;
        affix::AffixCatalog::load(config)?.pattern_suggestions(word)
    }

    pub(super) fn extension_patterns(&self, stem: &str) -> Result<Vec<AffixPattern>> {
        self.validate_word(stem)?;
        let config = self.config.as_ref().ok_or_else(|| {
            eyre!("Affix learning unavailable: no spelling configuration; plain-word acceptance is still available")
        })?;
        affix::AffixCatalog::load(config)?.extension_patterns(stem)
    }

    pub(super) fn stem_suggestions(&self, word: &str, flags: &str) -> Result<Vec<AffixSuggestion>> {
        self.validate_word(word)?;
        let config = self.config.as_ref().ok_or_else(|| {
            eyre!("Affix learning unavailable: no spelling configuration; plain-word acceptance is still available")
        })?;
        affix::AffixCatalog::load(config)?.stem_suggestions(word, flags)
    }

    pub(super) fn preview_entry(&self, stem: &str, flags: Option<&str>) -> Result<AffixSuggestion> {
        self.validate_word(stem)?;
        if flags.is_none() {
            return Ok(AffixSuggestion {
                entry: DictionaryEntry::plain(stem)?,
                forms: vec![stem.to_owned()],
            });
        }
        let config = self.config.as_ref().ok_or_else(|| {
            eyre!("Affix learning unavailable: no spelling configuration; plain-word acceptance is still available")
        })?;
        affix::expand(config, stem, flags)
    }

    pub(super) fn record_entry(
        &mut self,
        entry: DictionaryEntry,
        confirmed_forms: &[String],
    ) -> Result<()> {
        let validated = match &self.config {
            Some(config) => affix::validate_entry(config, &entry)?,
            None if entry.flags().is_some() => {
                bail!("Affix learning unavailable: no spelling configuration; plain-word acceptance is still available");
            }
            None => self.preview_entry(entry.word(), entry.flags())?,
        };
        if entry.flags().is_some()
            && confirmed_forms.iter().collect::<BTreeSet<_>>()
                != validated.forms.iter().collect::<BTreeSet<_>>()
        {
            bail!("Confirmed affix forms do not match the regenerated expansion; preview the entry again");
        }
        self.validate_word(validated.entry.word())?;
        for form in &validated.forms {
            self.validate_word(form)?;
        }

        self.remember_entry(validated.entry, validated.forms.into_iter().collect());
        Ok(())
    }

    fn remember_entry(&mut self, entry: DictionaryEntry, forms: BTreeSet<String>) {
        if entry.flags().is_some() {
            self.pending_entries
                .retain(|pending, _| pending.flags().is_some() || pending.word() != entry.word());
        } else if self.knows_word(entry.word()) {
            return;
        }
        // Reconfirmation replaces this entry's authority, but never revokes words
        // already accepted during the session (even if its expansion has changed).
        self.accepted_words.extend(forms.iter().cloned());
        self.pending_entries.insert(entry, forms);
    }

    /// Re-validate confirmed affixes before any writes, then update sources and vocabulary,
    /// read back entries, and refresh and verify engines.
    /// Failures stop the sequence; this is not a cross-file transaction.
    pub(super) fn commit(
        self,
        changes: UserPicked,
        sources: &mut dyn SourceUpdates,
        dictionaries: &mut dyn DictionaryReload,
    ) -> Result<()> {
        let mut catalog = None;
        for (entry, confirmed) in self
            .pending_entries
            .iter()
            .filter(|(entry, _)| entry.flags().is_some())
        {
            let config = self.config.as_ref().ok_or_else(|| {
                eyre!("Cannot revalidate affix forms without a spelling configuration; preview the entry again")
            })?;
            let regenerated = (|| -> Result<AffixSuggestion> {
                let catalog = match &mut catalog {
                    Some(catalog) => catalog,
                    slot @ None => slot.insert(affix::AffixCatalog::load(config)?),
                };
                catalog.expand(entry.word(), entry.flags())
            })()
            .wrap_err_with(|| {
                format!(
                    "Could not revalidate confirmed affix entry {} before commit; preview the entry again",
                    entry.to_line()
                )
            })?;
            if regenerated.forms.into_iter().collect::<BTreeSet<_>>() != *confirmed {
                bail!(
                    "Confirmed affix forms for {} changed before commit; preview the entry again",
                    entry.to_line()
                );
            }
        }
        let entries: BTreeSet<_> = self.pending_entries.into_keys().collect();
        let words = self.accepted_words;
        sources.apply_source_changes(changes)?;
        if let Some(store) = self.store.filter(|_| !entries.is_empty()) {
            store.update_entries(&entries)?;
            let persisted = store.read_entries()?;
            if !entries
                .iter()
                .all(|entry| persisted.contains(&entry.to_line()))
            {
                bail!("Dictionary update did not retain every accepted word or confirmed entry");
            }
            if let Some(path) = self.learning_path.as_deref() {
                dictionaries.include_learning_dictionary(path)?;
            }
            dictionaries.reload_dictionaries()?;
            dictionaries.verify_words(&words)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
