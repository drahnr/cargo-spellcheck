//! Suffix metadata discovery and engine-owned expansion for vocabulary learning.
//!
//! We index identifiers and strip alternatives, never apply affix rules ourselves.
//! Every preview and proposed shortened stem is checked by ZSpell's expansion engine.

use super::entry::{validate_word, DictionaryEntry};
use crate::errors::*;
use crate::Config;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct AffixSuggestion {
    pub entry: DictionaryEntry,
    pub forms: Vec<String>,
}

#[derive(Debug)]
pub(super) struct AffixCatalog {
    #[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
    inner: engine::Catalog,
}

impl AffixCatalog {
    pub(super) fn load(config: &Config) -> Result<Self> {
        #[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
        {
            Ok(Self {
                inner: engine::Catalog::load(config)?,
            })
        }
        #[cfg(not(any(feature = "hunspell", feature = "zet", feature = "spellbook")))]
        {
            let _ = config;
            bail!("Affix suggestions unavailable: no compiled spelling backend");
        }
    }

    pub(super) fn suggestions(&self, word: &str) -> Result<Vec<AffixSuggestion>> {
        validate_word(word)?;
        #[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
        {
            let mut suggestions = Vec::new();
            for suggestion in self.inner.expansions(word)? {
                // An inflected surface can fail a suffix's stem condition. Keep
                // that suffix selectable when engine-validated shortening works.
                let flags = suggestion.entry.flags().expect("catalog suffix");
                if suggestion.forms.iter().any(|form| form != word)
                    || self.stem_suggestions(word, flags)?.len() > 1
                {
                    suggestions.push(suggestion);
                }
            }
            Ok(suggestions)
        }
        #[cfg(not(any(feature = "hunspell", feature = "zet", feature = "spellbook")))]
        bail!("Affix suggestions unavailable: no compiled spelling backend")
    }

    #[cfg_attr(
        not(any(feature = "hunspell", feature = "zet", feature = "spellbook")),
        allow(dead_code)
    )]
    pub(super) fn stem_suggestions(&self, word: &str, flags: &str) -> Result<Vec<AffixSuggestion>> {
        validate_word(word)?;
        #[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
        {
            self.inner.stem_suggestions(word, flags)
        }
        #[cfg(not(any(feature = "hunspell", feature = "zet", feature = "spellbook")))]
        {
            let _ = flags;
            bail!("Affix suggestions unavailable: no compiled spelling backend");
        }
    }

    pub(super) fn expand(&self, word: &str, flags: Option<&str>) -> Result<AffixSuggestion> {
        validate_word(word)?;
        let Some(flags) = flags else {
            return plain(word);
        };
        #[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
        {
            self.inner.expand(word, flags)
        }
        #[cfg(not(any(feature = "hunspell", feature = "zet", feature = "spellbook")))]
        {
            let _ = flags;
            bail!("Affix expansion unavailable: no compiled spelling backend");
        }
    }
}

fn plain(word: &str) -> Result<AffixSuggestion> {
    Ok(AffixSuggestion {
        entry: DictionaryEntry::plain(word)?,
        forms: vec![word.to_owned()],
    })
}

pub(super) fn suggestions(config: &Config, word: &str) -> Result<Vec<AffixSuggestion>> {
    validate_word(word)?;
    AffixCatalog::load(config)?.suggestions(word)
}

pub(super) fn expand(config: &Config, word: &str, flags: Option<&str>) -> Result<AffixSuggestion> {
    validate_word(word)?;
    if flags.is_none() {
        return plain(word);
    }
    AffixCatalog::load(config)?.expand(word, flags)
}

/// Re-resolve dictionaries when recording: previews are not authority to persist stale flags.
pub(super) fn validate_entry(config: &Config, entry: &DictionaryEntry) -> Result<AffixSuggestion> {
    expand(config, entry.word(), entry.flags())
}

#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
mod engine {
    use super::*;
    use crate::checker::dictaffix::DicAff;
    use std::collections::{BTreeMap, BTreeSet};

    #[derive(Clone, Copy, Debug, Default)]
    enum FlagMode {
        #[default]
        Short,
        Long,
        Numeric,
        Utf8,
    }

    impl FlagMode {
        fn flag(self, text: &str) -> Result<String> {
            let safe = !text.is_empty()
                && !text.chars().any(|ch| {
                    ch.is_whitespace()
                        || ch.is_control()
                        || matches!(ch, '/' | '\\' | '#' | '\u{feff}')
                });
            let valid = safe
                && match self {
                    Self::Short => text.len() == 1 && text.is_ascii(),
                    Self::Long => text.len() == 2 && text.is_ascii(),
                    Self::Numeric => {
                        text.bytes().all(|ch| ch.is_ascii_digit())
                            && text.parse::<u16>().is_ok_and(|flag| flag != 0)
                    }
                    Self::Utf8 => {
                        text.chars().count() == 1
                            && text.chars().all(|ch| ch != '\0' && u32::from(ch) <= 0xffff)
                    }
                };
            if !valid {
                bail!("Invalid suffix flag {text:?} for {self:?} FLAG mode: expected one flag identifier");
            }
            Ok(match self {
                Self::Numeric => text.parse::<u16>()?.to_string(),
                _ => text.to_owned(),
            })
        }

        /// Index references only; compound matching remains the spelling engine's job.
        fn compound_flags(self, pattern: &str) -> Result<BTreeSet<String>> {
            let mut remaining = pattern;
            let mut flags = BTreeSet::new();
            while !remaining.is_empty() {
                let (text, rest) = match self {
                    Self::Short | Self::Utf8 => {
                        let ch = remaining.chars().next().expect("nonempty pattern");
                        if matches!(ch, '*' | '?') {
                            bail!("Malformed COMPOUNDRULE {pattern:?}: operator without a flag");
                        }
                        remaining.split_at(ch.len_utf8())
                    }
                    Self::Long | Self::Numeric => remaining
                        .strip_prefix('(')
                        .and_then(|group| group.split_once(')'))
                        .ok_or_else(|| eyre!("Malformed COMPOUNDRULE {pattern:?}: expected parenthesized flag identifiers for {self:?} FLAG mode"))?,
                };
                flags.insert(
                    self.flag(text)
                        .wrap_err("Invalid COMPOUNDRULE flag reference")?,
                );
                remaining = rest.strip_prefix(['*', '?']).unwrap_or(rest);
            }
            if flags.is_empty() {
                bail!("Malformed COMPOUNDRULE: empty pattern");
            }
            Ok(flags)
        }

        fn directive(self) -> &'static str {
            match self {
                Self::Short => "",
                Self::Long => "FLAG long\n",
                Self::Numeric => "FLAG num\n",
                Self::Utf8 => "FLAG UTF-8\n",
            }
        }
    }

    #[derive(Debug)]
    struct Suffix {
        aff: String,
        strips: BTreeSet<String>,
    }

    #[derive(Debug)]
    pub(super) struct Catalog {
        mode: FlagMode,
        explicit_utf8: bool,
        suffixes: BTreeMap<String, Suffix>,
        known_stems: BTreeSet<String>,
    }

    impl Catalog {
        pub(super) fn load(config: &Config) -> Result<Self> {
            let active = [
                (
                    cfg!(feature = "hunspell"),
                    "Hunspell",
                    config.hunspell.as_ref(),
                ),
                (
                    cfg!(feature = "spellbook"),
                    "Spellbook",
                    config.spellbook.as_ref(),
                ),
                (cfg!(feature = "zet"), "ZSpell", config.zet.as_ref()),
            ];
            let mut resolved: Option<(String, String)> = None;
            let mut known_stems = BTreeSet::new();
            let mut language = None;
            for (compiled, name, spelling) in active {
                let Some(spelling) = spelling.filter(|_| compiled) else {
                    continue;
                };
                if language.is_some_and(|lang| lang != spelling.lang) {
                    bail!("Incompatible affix definitions: active spelling backends use different languages");
                }
                language = Some(spelling.lang);
                let data = DicAff::load(
                    &spelling.extra_dictionaries,
                    &spelling.search_dirs,
                    spelling.lang,
                    spelling.use_builtin,
                    spelling.skip_os_lookups,
                )
                .map_err(|error| {
                    eyre!("Affix suggestions unavailable: could not load {name} dictionaries: {error}")
                })?;
                let definition = definition(&data.aff);
                if let Some((first_name, first_aff)) = &resolved {
                    if *first_aff != definition {
                        bail!("Incompatible affix definitions for {first_name} and {name}: shared learned flags require matching affix files");
                    }
                } else {
                    resolved = Some((name.to_owned(), definition));
                }
                for line in data.dic.lines().skip(1) {
                    let stem = line.split(['/', ' ', '\t']).next().unwrap_or("");
                    if validate_word(stem).is_ok() {
                        known_stems.insert(stem.to_owned());
                    }
                }
            }
            let (_, aff) = resolved.ok_or_else(|| {
                eyre!("Affix suggestions unavailable: no active spelling backend")
            })?;
            Self::from_aff(&aff, known_stems)
        }

        fn from_aff(aff: &str, known_stems: BTreeSet<String>) -> Result<Self> {
            let lines = aff.lines().collect::<Vec<_>>();
            let mut mode = FlagMode::Short;
            let mut has_encoding = false;
            let mut special = BTreeSet::new();
            let mut prefix_flags = BTreeSet::new();
            // Metadata only: rule conditions and additions are passed untouched to ZSpell.
            for line in &lines {
                let fields = line.split_whitespace().collect::<Vec<_>>();
                let Some(&key) = fields.first() else { continue };
                match key {
                    "SET" => {
                        has_encoding = true;
                        if !matches!(fields.get(1), Some(&"UTF8" | &"UTF-8")) {
                            bail!("Unsupported affix encoding: suffix learning requires SET UTF-8");
                        }
                    }
                    "FLAG" => {
                        mode = match fields.get(1).copied() {
                            Some("long") => FlagMode::Long,
                            Some("num") => FlagMode::Numeric,
                            Some("UTF-8" | "UTF8") => FlagMode::Utf8,
                            other => bail!("Unsupported FLAG mode {other:?}"),
                        };
                    }
                    "PFX" if fields.len() == 4 && matches!(fields[2], "Y" | "N") => {
                        prefix_flags.insert(fields[1].to_owned());
                    }
                    "AF" | "AM" => bail!("Unsupported affix directive {key}: flag/morphology aliases are not supported by suffix learning"),
                    "IGNORE" | "ICONV" | "OCONV" | "FULLSTRIP" | "COMPLEXPREFIXES" => {
                        bail!("Unsupported affix directive {key}: ZSpell suffix previews cannot preserve its semantics");
                    }
                    "NOSUGGEST" | "FORBIDDENWORD" | "NEEDAFFIX" | "PSEUDOROOT"
                    | "CIRCUMFIX" | "KEEPCASE" | "ONLYINCOMPOUND" | "SUBSTANDARD"
                    | "WARN" | "FORCEUCASE" | "COMPOUNDFLAG" | "COMPOUNDBEGIN"
                    | "COMPOUNDMIDDLE" | "COMPOUNDEND" | "COMPOUNDROOT"
                    | "COMPOUNDPERMITFLAG" | "COMPOUNDFORBIDFLAG" | "COMPOUNDFORCEUPPER" => {
                        if let Some(flag) = fields.get(1) {
                            special.insert((*flag).to_owned());
                        }
                    }
                    _ => {}
                }
            }
            if !has_encoding && !aff.is_ascii() {
                bail!("Unsupported affix encoding: non-ASCII affixes must declare SET UTF-8");
            }
            let special = special
                .iter()
                .map(|flag| mode.flag(flag))
                .collect::<Result<BTreeSet<_>>>()?;
            let prefix_flags = prefix_flags
                .iter()
                .map(|flag| mode.flag(flag))
                .collect::<Result<BTreeSet<_>>>()?;
            let compound_flags = compound_rule_flags(&lines, mode)?;
            let mut suffixes = BTreeMap::new();
            let mut index = 0;
            while index < lines.len() {
                let fields = lines[index].split_whitespace().collect::<Vec<_>>();
                index += 1;
                if fields.first() != Some(&"SFX") {
                    continue;
                }
                if fields.len() != 4 || !matches!(fields[2], "Y" | "N") {
                    bail!("Malformed SFX header: expected SFX flag Y/N count");
                }
                let flag = mode.flag(fields[1])?;
                if prefix_flags.contains(&flag) {
                    bail!("Unsupported shared PFX/SFX flag {flag}: a suffix-only preview would omit prefix forms");
                }
                if compound_flags.contains(&flag) {
                    bail!("Unsupported SFX flag {flag} referenced by COMPOUNDRULE: a suffix-only preview would omit compound forms");
                }
                let count = fields[3]
                    .parse::<usize>()
                    .wrap_err("Invalid SFX rule count")?;
                if count > lines.len().saturating_sub(index) {
                    bail!("Malformed SFX {flag}: missing rule lines");
                }
                let mut suffix = Suffix {
                    aff: format!("SET UTF-8\n{}{}\n", mode.directive(), lines[index - 1]),
                    strips: BTreeSet::from([String::new()]),
                };
                for line in &lines[index..index + count] {
                    let rule = line.split_whitespace().collect::<Vec<_>>();
                    if rule.len() < 5 || rule[0] != "SFX" || mode.flag(rule[1])? != flag {
                        bail!("Malformed SFX {flag}: expected matching rule with a condition");
                    }
                    if rule[3].contains('/') {
                        bail!("Unsupported SFX {flag} continuation classes: ZSpell cannot expand them faithfully");
                    }
                    // ZSpell 0.5.5 treats the zero-addition marker as a literal '0'.
                    if rule[3] == "0" {
                        bail!("Unsupported SFX {flag} zero addition: ZSpell 0.5.5 cannot expand it faithfully");
                    }
                    if rule[2] != "0" {
                        suffix.strips.insert(rule[2].to_owned());
                    }
                    suffix.aff.push_str(line);
                    suffix.aff.push('\n');
                }
                index += count;
                if count != 0
                    && !special.contains(&flag)
                    && suffixes.insert(flag.clone(), suffix).is_some()
                {
                    bail!("Duplicate SFX flag {flag}");
                }
            }
            let catalog = Self {
                mode,
                explicit_utf8: has_encoding,
                suffixes,
                known_stems,
            };
            // Make engine parsing failures visible even before the first preview.
            for suffix in catalog.suffixes.values() {
                let _ = zspell::builder()
                    .config_str(&suffix.aff)
                    .dict_str("0\n")
                    .build()
                    .wrap_err("Could not parse suffix rules with the ZSpell expansion engine")?;
            }
            Ok(catalog)
        }

        fn suffix(&self, flags: &str) -> Result<(String, &Suffix)> {
            let flag = self.mode.flag(flags)?;
            let suffix = self.suffixes.get(&flag).ok_or_else(|| {
                eyre!("Unknown or nonproductive suffix flag {flags:?}: choose a suffix from this dictionary's catalog")
            })?;
            Ok((flag, suffix))
        }

        pub(super) fn expand(&self, word: &str, flags: &str) -> Result<AffixSuggestion> {
            validate_word(word)?;
            if word.contains('#') {
                bail!(
                    "Unsupported dictionary syntax in stem: ZSpell treats '#' as a comment marker"
                );
            }
            if !self.explicit_utf8 && !word.is_ascii() {
                bail!("Unsupported affix encoding: non-ASCII learned stems require SET UTF-8");
            }
            let (flag, suffix) = self.suffix(flags)?;
            let dic = format!("1\n{word}/{flag}\n");
            let dictionary = zspell::builder()
                .config_str(&suffix.aff)
                .dict_str(&dic)
                .build()
                .wrap_err("Could not expand the selected suffix with ZSpell")?;
            let forms = dictionary
                .wordlist()
                .inner()
                .keys()
                .chain(dictionary.wordlist_nosuggest().inner().keys())
                .filter(|form| {
                    !dictionary
                        .wordlist_forbidden()
                        .inner()
                        .contains_key(form.as_ref())
                })
                .map(|form| form.to_string())
                .collect::<BTreeSet<_>>();
            for form in &forms {
                validate_word(form)
                    .wrap_err("Suffix expansion produced a word that cannot be learned")?;
            }
            Ok(AffixSuggestion {
                entry: DictionaryEntry::flagged(word, &flag)?,
                forms: forms.into_iter().collect(),
            })
        }

        pub(super) fn expansions(&self, word: &str) -> Result<Vec<AffixSuggestion>> {
            self.suffixes
                .keys()
                .map(|flag| self.expand(word, flag))
                .collect()
        }

        pub(super) fn stem_suggestions(
            &self,
            word: &str,
            flags: &str,
        ) -> Result<Vec<AffixSuggestion>> {
            let (_, suffix) = self.suffix(flags)?;
            let mut candidates = BTreeSet::new();
            for boundary in word
                .char_indices()
                .map(|(index, _)| index)
                .chain(std::iter::once(word.len()))
            {
                for strip in &suffix.strips {
                    let stem = format!("{}{strip}", &word[..boundary]);
                    if stem != word && validate_word(&stem).is_ok() {
                        candidates.insert(stem);
                    }
                }
            }
            let mut suggestions = Vec::new();
            for stem in candidates {
                let suggestion = self.expand(&stem, flags)?;
                if suggestion.forms.iter().any(|form| form == word) {
                    suggestions.push(suggestion);
                }
            }
            suggestions.sort_by(|left, right| {
                let rank = |suggestion: &AffixSuggestion| {
                    (
                        !self.known_stems.contains(suggestion.entry.word()),
                        suggestion.entry.word().chars().count(),
                    )
                };
                rank(left)
                    .cmp(&rank(right))
                    .then_with(|| left.entry.word().cmp(right.entry.word()))
            });
            suggestions.push(self.expand(word, flags)?);
            Ok(suggestions)
        }
    }

    fn compound_rule_flags(lines: &[&str], mode: FlagMode) -> Result<BTreeSet<String>> {
        let mut flags = BTreeSet::new();
        let mut index = 0;
        while index < lines.len() {
            let fields = lines[index].split_whitespace().collect::<Vec<_>>();
            index += 1;
            if fields.first() != Some(&"COMPOUNDRULE") {
                continue;
            }
            let count = fields
                .get(1)
                .filter(|_| fields.len() == 2)
                .ok_or_else(|| eyre!("Malformed COMPOUNDRULE header: expected a rule count"))?
                .parse::<usize>()
                .wrap_err("Malformed COMPOUNDRULE rule count")?;
            if count > lines.len().saturating_sub(index) {
                bail!("Malformed COMPOUNDRULE: missing rule lines");
            }
            // Consume the counted table, so a numeric header is never mistaken
            // for a default-mode numeric character flag in a rule pattern.
            for line in &lines[index..index + count] {
                let rule = line.split_whitespace().collect::<Vec<_>>();
                if rule.len() != 2 || rule[0] != "COMPOUNDRULE" {
                    bail!("Malformed COMPOUNDRULE: expected a rule pattern");
                }
                flags.extend(mode.compound_flags(rule[1])?);
            }
            index += count;
        }
        Ok(flags)
    }

    /// Ignore layout/comments, but conservatively compare all semantic directives,
    /// including FLAG/SET and otherwise unrelated directives that could alter checking.
    fn definition(aff: &str) -> String {
        aff.trim_start_matches('\u{feff}')
            .lines()
            .map(|line| line.split_once('#').map_or(line, |(body, _)| body))
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_backend() -> Config {
        Config {
            hunspell: None,
            spellbook: None,
            zet: None,
            ..Config::default()
        }
    }

    #[test]
    fn plain_entries_need_no_dictionary_or_backend() {
        let config = no_backend();
        let entry = DictionaryEntry::plain("widget").unwrap();
        assert_eq!(
            validate_entry(&config, &entry).unwrap(),
            expand(&config, "widget", None).unwrap()
        );
        assert_eq!(expand(&config, "widget", None).unwrap().forms, ["widget"]);
        for word in ["word/S", "two words", "word\nInjected"] {
            assert!(expand(&config, word, None)
                .unwrap_err()
                .to_string()
                .contains("plain Hunspell word"));
            assert!(suggestions(&config, word)
                .unwrap_err()
                .to_string()
                .contains("plain Hunspell word"));
        }
        assert!(AffixCatalog::load(&config)
            .err()
            .unwrap()
            .to_string()
            .contains("spelling backend"));
    }

    #[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
    mod catalog {
        use super::*;
        use std::path::PathBuf;

        const RULES: &str = "SET UTF-8\nSFX S Y 1\nSFX S 0 s .\nSFX D Y 2\nSFX D 0 d e\nSFX D 0 ed [^e]\nSFX Y N 1\nSFX Y y ies [^aeiou]y\n";

        struct Fixture(PathBuf);
        impl Fixture {
            fn new(aff: &str) -> Self {
                let path = std::env::temp_dir()
                    .join(format!("spellcheck-affix-catalog-{}", uuid::Uuid::new_v4()));
                fs_err::create_dir(&path).unwrap();
                fs_err::write(path.join("en_US.aff"), aff).unwrap();
                fs_err::write(path.join("en_US.dic"), "2\nTest\nfoobar\n").unwrap();
                Self(path)
            }

            fn spelling(&self) -> crate::HunspellConfig {
                crate::HunspellConfig {
                    search_dirs: vec![self.0.clone()].into(),
                    use_builtin: false,
                    skip_os_lookups: true,
                    ..Default::default()
                }
            }

            fn config(&self) -> Config {
                let mut config = no_backend();
                if cfg!(feature = "hunspell") {
                    config.hunspell = Some(self.spelling());
                } else if cfg!(feature = "spellbook") {
                    config.spellbook = Some(self.spelling());
                } else {
                    config.zet = Some(self.spelling());
                }
                config
            }
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = fs_err::remove_dir_all(&self.0);
            }
        }

        #[test]
        fn suffix_catalog_uses_engine_conditions_and_includes_flags_absent_from_dic() {
            let fixture = Fixture::new(RULES);
            let catalog = AffixCatalog::load(&fixture.config()).unwrap();
            let suggestions = catalog.suggestions("foobar").unwrap();
            assert_eq!(
                suggestions
                    .iter()
                    .map(|s| s.entry.flags().unwrap())
                    .collect::<Vec<_>>(),
                ["D", "S"]
            );
            assert_eq!(
                catalog.expand("foobar", Some("S")).unwrap().forms,
                ["foobar", "foobars"]
            );
            assert_eq!(
                catalog.expand("pony", Some("Y")).unwrap().forms,
                ["ponies", "pony"]
            );
            assert_eq!(catalog.expand("key", Some("Y")).unwrap().forms, ["key"]);
            assert!(catalog.expand("foobar", Some("X")).is_err());
            assert!(catalog
                .expand("foo#bar", Some("S"))
                .unwrap_err()
                .to_string()
                .contains("comment marker"));
            assert!(catalog.expand("foobar", Some("SD")).is_err());
            assert!(catalog.expand("foobar", Some("S\nInjected")).is_err());
        }

        #[test]
        fn shortening_and_strip_restoration_are_validated_by_expansion() {
            let fixture = Fixture::new(&format!("{RULES}SFX E Y 1\nSFX E e ing e\n"));
            let catalog = AffixCatalog::load(&fixture.config()).unwrap();
            let choices = catalog.stem_suggestions("foobars", "S").unwrap();
            assert_eq!(choices.first().unwrap().entry.to_line(), "foobar/S");
            assert_eq!(choices.last().unwrap().entry.to_line(), "foobars/S");
            assert!(choices[..choices.len() - 1]
                .iter()
                .all(|s| s.forms.contains(&"foobars".to_owned())));
            let choices = catalog.stem_suggestions("making", "E").unwrap();
            assert!(choices
                .iter()
                .any(|s| s.entry.to_line() == "make/E" && s.forms.contains(&"making".to_owned())));
            assert_eq!(
                catalog.expand("hope", Some("D")).unwrap().forms,
                ["hope", "hoped"]
            );
            assert!(catalog
                .stem_suggestions("hoped", "D")
                .unwrap()
                .iter()
                .any(|suggestion| suggestion.entry.to_line() == "hope/D"));
            assert!(catalog
                .suggestions("ponies")
                .unwrap()
                .iter()
                .any(|suggestion| suggestion.entry.flags() == Some("Y")));
            assert_eq!(
                catalog.stem_suggestions("cafés", "S").unwrap()[0]
                    .entry
                    .word(),
                "café"
            );
            let selected = catalog.expand("foobaz", Some("S")).unwrap();
            assert_eq!(selected.entry.to_line(), "foobaz/S");
            assert_eq!(
                validate_entry(&fixture.config(), &selected.entry).unwrap(),
                selected
            );
        }

        #[test]
        fn numeric_long_and_unicode_flag_identifiers_are_not_split() {
            for (mode, flag) in [("num", "123"), ("long", "AB"), ("UTF-8", "é")] {
                let fixture = Fixture::new(&format!(
                    "SET UTF-8\nFLAG {mode}\nSFX {flag} Y 1\nSFX {flag} 0 s .\n"
                ));
                let catalog = AffixCatalog::load(&fixture.config()).unwrap();
                let suggestion = catalog.suggestions("widget").unwrap().pop().unwrap();
                assert_eq!(suggestion.entry.flags(), Some(flag));
                assert_eq!(suggestion.forms, ["widget", "widgets"]);
                assert_eq!(
                    catalog.stem_suggestions("widgets", flag).unwrap()[0]
                        .entry
                        .word(),
                    "widget"
                );
            }
        }

        #[test]
        fn unsupported_semantics_have_explicit_diagnostics() {
            for (aff, expected) in [
                (format!("{RULES}AF 1\nAF S\n"), "aliases"),
                (
                    "SET UTF-8\nSFX S Y 1\nSFX S 0 s/D .\n".into(),
                    "continuation",
                ),
                ("SET ISO8859-1\n".into(), "encoding"),
                (format!("{RULES}IGNORE x\n"), "IGNORE"),
                (
                    format!("{RULES}PFX S Y 1\nPFX S 0 un .\n"),
                    "shared PFX/SFX",
                ),
                (
                    "SET UTF-8\nSFX S Y 1\nSFX S s 0 s\n".into(),
                    "zero addition",
                ),
                (
                    "SET UTF-8\nFLAG long\nSFX S Y 1\nSFX S 0 s .\n".into(),
                    "flag",
                ),
            ] {
                let fixture = Fixture::new(&aff);
                let error = AffixCatalog::load(&fixture.config())
                    .err()
                    .unwrap()
                    .to_string();
                assert!(error.contains(expected), "expected {expected:?} in {error}");
            }
        }

        #[test]
        fn unrelated_compound_and_special_flags_are_not_suffix_choices() {
            let fixture = Fixture::new(&format!("{RULES}PFX U Y 1\nPFX U 0 un .\nONLYINCOMPOUND c\nCOMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE c*\nNOSUGGEST !\n"));
            let catalog = AffixCatalog::load(&fixture.config()).unwrap();
            assert!(catalog.expand("word", Some("c")).is_err());
            assert!(catalog.expand("word", Some("!")).is_err());
            assert!(catalog.expand("word", Some("U")).is_err());
            assert_eq!(
                catalog.expand("word", Some("S")).unwrap().forms,
                ["word", "words"]
            );
        }

        #[test]
        fn compound_rule_suffix_overlap_is_rejected_in_every_flag_mode() {
            for (mode, flag, overlap, unrelated) in [
                ("", "S", "SS", "X*Y?"),
                ("", "2", "2*2?", "X*Y?"),
                ("FLAG UTF-8\n", "é", "é*β?", "β?γ*"),
                ("FLAG long\n", "AB", "(AB)(CD)?", "(CD)*(EF)?"),
                ("FLAG num\n", "123", "(0123)*(456)?", "(456)?(789)*"),
            ] {
                let aff = |pattern: &str| {
                    format!(
                    "SET UTF-8\n{mode}COMPOUNDRULE 2\nCOMPOUNDRULE {pattern}\nCOMPOUNDRULE {unrelated}\nSFX {flag} Y 1\nSFX {flag} 0 s .\n"
                )
                };
                let unsafe_fixture = Fixture::new(&aff(overlap));
                let error = AffixCatalog::load(&unsafe_fixture.config())
                    .unwrap_err()
                    .to_string();
                assert!(error.contains("Unsupported SFX flag"), "{error}");
                assert!(error.contains("COMPOUNDRULE"), "{error}");
                let safe_fixture = Fixture::new(&aff(unrelated));
                let catalog = AffixCatalog::load(&safe_fixture.config()).unwrap();
                assert_eq!(
                    catalog.expand("widget", Some(flag)).unwrap().forms,
                    ["widget", "widgets"]
                );
            }
        }

        #[test]
        fn malformed_compound_rule_metadata_cannot_hide_flag_references() {
            for (mode, pattern) in [
                ("", "*S"),
                ("FLAG UTF-8\n", "é??"),
                ("FLAG long\n", "AB"),
                ("FLAG num\n", "(123"),
            ] {
                let fixture = Fixture::new(&format!(
                    "{RULES}{mode}COMPOUNDRULE 1\nCOMPOUNDRULE {pattern}\n"
                ));
                assert!(AffixCatalog::load(&fixture.config())
                    .unwrap_err()
                    .to_string()
                    .contains("COMPOUNDRULE"));
            }
        }

        #[test]
        fn builtin_english_has_real_suffix_options() {
            let mut config = no_backend();
            let spelling = crate::HunspellConfig {
                use_builtin: true,
                skip_os_lookups: true,
                search_dirs: Vec::<PathBuf>::new().into(),
                ..Default::default()
            };
            if cfg!(feature = "hunspell") {
                config.hunspell = Some(spelling);
            } else if cfg!(feature = "spellbook") {
                config.spellbook = Some(spelling);
            } else {
                config.zet = Some(spelling);
            }
            let catalog = AffixCatalog::load(&config).unwrap();
            let suggestions = catalog.suggestions("foobarquux").unwrap();
            assert!(!suggestions.is_empty());
            let plural = suggestions
                .iter()
                .find(|s| s.entry.flags() == Some("S"))
                .unwrap();
            assert!(plural.forms.contains(&"foobarquuxes".to_owned()));
            assert!(catalog
                .stem_suggestions("foobars", "S")
                .unwrap()
                .iter()
                .any(|s| s.entry.word() == "foobar"));
        }

        #[cfg(any(
            all(feature = "hunspell", feature = "spellbook"),
            all(feature = "hunspell", feature = "zet"),
            all(feature = "spellbook", feature = "zet")
        ))]
        #[test]
        fn mixed_backend_affix_definitions_must_match_even_for_record_validation() {
            let first = Fixture::new(RULES);
            let second = Fixture::new(&RULES.replace("0 s .", "0 z ."));
            let mut config = no_backend();
            if cfg!(feature = "hunspell") {
                config.hunspell = Some(first.spelling());
                if cfg!(feature = "spellbook") {
                    config.spellbook = Some(second.spelling());
                } else {
                    config.zet = Some(second.spelling());
                }
            } else {
                config.spellbook = Some(first.spelling());
                config.zet = Some(second.spelling());
            }
            let entry = DictionaryEntry::with_test_flags("widget", "S").unwrap();
            assert!(validate_entry(&config, &entry)
                .unwrap_err()
                .to_string()
                .contains("Incompatible affix definitions"));
            assert_eq!(expand(&config, "widget", None).unwrap().forms, ["widget"]);
            fs_err::write(
                second.0.join("en_US.aff"),
                format!("# layout is not semantics\n{RULES}"),
            )
            .unwrap();
            assert_eq!(
                validate_entry(&config, &entry).unwrap().forms,
                ["widget", "widgets"]
            );
        }
    }
}
