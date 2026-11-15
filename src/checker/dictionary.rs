//! Backend-independent refresh of dictionaries after persistence or external edits.

use crate::errors::Result;

/// Register persisted learning dictionaries and explicitly re-read configured files.
///
/// Call after learned words have been persisted. Session acceptance belongs to the
/// learner's shared overlay, not individual engines; checking does not reload files.
/// A failed reload leaves the existing checker state usable and unchanged.
///
/// `Checkers` currently reconstructs all active engines, including NLP, and only
/// installs the replacement once construction succeeds.
pub(crate) trait DictionaryReload {
    /// Register a persisted dictionary for the next reload without mutating live engines.
    fn include_learning_dictionary(&mut self, _path: &std::path::Path) -> Result<()> {
        Ok(())
    }

    fn reload_dictionaries(&mut self) -> Result<()>;

    /// Verify that every supplied word is recognized by every active spelling engine.
    ///
    /// Call after reloading to check standalone word acceptance, including dictionary
    /// flag restrictions. Grammar engines are ignored. This does not reload files or
    /// roll back engine state or persisted dictionaries if verification fails.
    fn verify_words(&self, words: &std::collections::BTreeSet<String>) -> Result<()>;
}
