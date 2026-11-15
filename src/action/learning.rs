//! Backend-neutral session vocabulary and its explicit commit boundaries.

use super::{interactive::UserPicked, update::SourceUpdates};
use crate::checker::DictionaryReload;
use crate::errors::*;
use crate::{Config, Suggestion};
use std::collections::BTreeSet;

mod dictionary;

/// A vocabulary store owns its format, validation, and locked read-merge-write.
/// Engines consume its persisted file through their separate reload interface.
pub(super) trait DictionaryStore {
    fn validate_word(&self, word: &str) -> Result<()>;
    fn read(&self) -> Result<BTreeSet<String>>;
    fn update(&self, words: &BTreeSet<String>) -> Result<()>;
}

/// One shared overlay for all spelling backends; engine instances stay immutable
/// during interaction. Dropping this value without committing writes nothing.
#[derive(Default)]
pub(super) struct SessionResolutions {
    accepted_words: BTreeSet<String>,
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
            .is_some_and(|word| self.accepted_words.contains(word))
    }

    pub(super) fn accept(&mut self, suggestion: &Suggestion<'_>) -> Result<()> {
        let word = suggestion
            .checked_word
            .as_deref()
            .ok_or_else(|| eyre!("Cannot accept vocabulary without the original checked word"))?;
        self.validate_word(word)?;
        self.accepted_words.insert(word.to_owned());
        Ok(())
    }

    /// Sources first, then vocabulary persistence/read-back, then engine refresh and verification.
    /// Failures stop the sequence; this is not a cross-file transaction.
    pub(super) fn commit(
        self,
        changes: UserPicked,
        sources: &mut dyn SourceUpdates,
        dictionaries: &mut dyn DictionaryReload,
    ) -> Result<()> {
        sources.apply_source_changes(changes)?;
        if let Some(store) = self.store.filter(|_| !self.accepted_words.is_empty()) {
            {
                let _hat = crate::TinHat::on();
                store.update(&self.accepted_words)?;
                let persisted = store.read()?;
                if !self.accepted_words.is_subset(&persisted) {
                    bail!("Dictionary update did not retain every accepted word");
                }
            }
            dictionaries.reload_dictionaries()?;
            dictionaries.verify_words(&self.accepted_words)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
