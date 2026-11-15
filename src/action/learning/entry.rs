//! Confirmed dictionary entries, kept separate from untrusted plain-word input.

use crate::errors::*;

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct DictionaryEntry {
    word: String,
    flags: Option<String>,
}

impl DictionaryEntry {
    pub(crate) fn plain(word: impl Into<String>) -> Result<Self> {
        let word = word.into();
        validate_word(&word)?;
        Ok(Self { word, flags: None })
    }

    /// Only the learning backend may construct flagged entries, after catalog validation.
    #[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
    pub(super) fn flagged(word: &str, flags: &str) -> Result<Self> {
        let mut entry = Self::plain(word)?;
        if flags.is_empty()
            || flags.chars().any(|ch| {
                ch.is_whitespace() || ch.is_control() || matches!(ch, '/' | '\\' | '#' | '\u{feff}')
            })
        {
            bail!("Invalid dictionary flags: expected validated flag identifiers without dictionary syntax");
        }
        entry.flags = Some(flags.to_owned());
        Ok(entry)
    }

    // Storage fixtures deliberately bypass the backend's catalog validation.
    #[cfg(test)]
    pub(crate) fn with_test_flags(word: &str, flags: &str) -> Result<Self> {
        assert!(
            matches!(flags, "S" | "D"),
            "only storage-fixture flags are allowed"
        );
        let mut entry = Self::plain(word)?;
        entry.flags = Some(flags.to_owned());
        Ok(entry)
    }

    pub(crate) fn word(&self) -> &str {
        &self.word
    }

    pub(crate) fn flags(&self) -> Option<&str> {
        self.flags.as_deref()
    }

    pub(crate) fn to_line(&self) -> String {
        match self.flags() {
            Some(flags) => format!("{}/{flags}", self.word),
            None => self.word.clone(),
        }
    }
}

/// Accept a single literal word, never flags, morphology, or dictionary syntax.
/// Keep this policy and its diagnostic identical to dictionary::validate_word.
pub(crate) fn validate_word(word: &str) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_words_preserve_the_exact_validation_policy() {
        for word in [
            "",
            "42",
            "+42",
            "-42",
            "#comment",
            "word/S",
            "word\\S",
            "two words",
            "word\tpo:noun",
            "word\nInjected",
            "word\rInjected",
            "word\0",
            "\u{feff}word",
            "word\u{a0}tail",
            "word\u{7f}",
        ] {
            let error = DictionaryEntry::plain(word).unwrap_err().to_string();
            assert_eq!(error, format!(
                "Cannot accept {:?}: expected one nonnumeric plain Hunspell word without whitespace, control characters, flags, escapes, or dictionary syntax",
                word
            ));
        }
        // The existing policy uses i64 parsing, not a broader numeric heuristic.
        for word in [
            "café",
            "naïve",
            "🦀",
            "\u{10000}word",
            "word\u{ffe6}",
            "word\u{ffe7}",
            "word.",
            "can't",
            "well-known",
            "1st",
            "9223372036854775808",
        ] {
            let entry = DictionaryEntry::plain(word).unwrap();
            assert_eq!(entry.word(), word);
            assert_eq!(entry.flags(), None);
            assert_eq!(entry.to_line(), word);
        }
    }
}
