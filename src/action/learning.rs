//! Backend-neutral session vocabulary and its explicit commit boundaries.

use super::{interactive::UserPicked, update::SourceUpdates};
use crate::checker::DictionaryReload;
use crate::errors::*;
use crate::{Config, Suggestion};
use std::collections::{BTreeMap, BTreeSet};

mod affix;
mod dictionary;
mod entry;

pub(super) use affix::AffixSuggestion;
pub(super) use entry::DictionaryEntry;

/// A vocabulary store owns its format, validation, and locked read-merge-write.
/// Engines consume its persisted file through their separate reload interface.
pub(super) trait DictionaryStore {
    fn validate_word(&self, word: &str) -> Result<()>;
    fn read(&self) -> Result<BTreeSet<String>>;
    fn update(&self, words: &BTreeSet<String>) -> Result<()>;
    fn read_entries(&self) -> Result<BTreeSet<String>> {
        self.read()
    }

    fn update_entries(&self, entries: &BTreeSet<DictionaryEntry>) -> Result<()> {
        let words = entries
            .iter()
            .map(|entry| {
                if entry.flags().is_some() {
                    bail!("This vocabulary store does not support affix entries");
                }
                Ok(entry.word().to_owned())
            })
            .collect::<Result<BTreeSet<_>>>()?;
        self.update(&words)
    }
}

/// One shared overlay for all spelling backends; engine instances stay immutable
/// during interaction. Dropping this value without committing writes nothing.
#[derive(Default)]
pub(super) struct SessionResolutions {
    accepted_words: BTreeSet<String>,
    accepted_entries: BTreeSet<DictionaryEntry>,
    confirmed_affix_forms: BTreeMap<DictionaryEntry, BTreeSet<String>>,
    config: Option<Config>,
    store: Option<Box<dyn DictionaryStore>>,
}

impl SessionResolutions {
    pub(super) fn new(config: &Config) -> Result<Self> {
        let store = if let Some(path) = config.learning_dictionary()? {
            log::info!("Accepted words will be saved to {}", path.display());
            Some(Box::new(dictionary::DictionaryFile::new(path)) as Box<dyn DictionaryStore>)
        } else {
            log::info!("Accepted words are session-only; configure learning_dictionary on an enabled spelling backend to save them");
            None
        };
        Ok(Self {
            store,
            config: Some(config.clone()),
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
        if !self.knows_word(word) {
            self.accepted_entries.insert(DictionaryEntry::plain(word)?);
            self.accepted_words.insert(word.to_owned());
        }
        Ok(())
    }

    pub(super) fn is_persistent(&self) -> bool {
        self.store.is_some()
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

        if validated.entry.flags().is_some() {
            self.confirmed_affix_forms.insert(
                validated.entry.clone(),
                validated.forms.iter().cloned().collect(),
            );
            self.accepted_entries
                .remove(&DictionaryEntry::plain(validated.entry.word())?);
        } else if self.knows_word(validated.entry.word()) {
            return Ok(());
        }
        self.accepted_words.extend(validated.forms);
        self.accepted_entries.insert(validated.entry);
        Ok(())
    }

    /// Revalidate confirmed affixes before any writes, then update sources and vocabulary,
    /// read back entries, and refresh and verify engines.
    /// Failures stop the sequence; this is not a cross-file transaction.
    pub(super) fn commit(
        self,
        changes: UserPicked,
        sources: &mut dyn SourceUpdates,
        dictionaries: &mut dyn DictionaryReload,
    ) -> Result<()> {
        for entry in self
            .accepted_entries
            .iter()
            .filter(|entry| entry.flags().is_some())
        {
            let confirmed = self.confirmed_affix_forms.get(entry).ok_or_else(|| {
                eyre!(
                    "Missing confirmed affix forms for {}; preview the entry again",
                    entry.to_line()
                )
            })?;
            let config = self.config.as_ref().ok_or_else(|| {
                eyre!("Cannot revalidate affix forms without a spelling configuration; preview the entry again")
            })?;
            let regenerated = affix::validate_entry(config, entry).wrap_err_with(|| {
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
        let entries = self.accepted_entries;
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
            dictionaries.reload_dictionaries()?;
            dictionaries.verify_words(&words)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
