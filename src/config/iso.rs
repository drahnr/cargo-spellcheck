//! Abstracts the combination of language code and country code
//! into one convenient type.
//!
//! Language code follows the [ISO 639-1](https://en.wikipedia.org/wiki/ISO_639-1) format.
//! Country code follows the [Alpha-2 ISO_3166-1](https://en.wikipedia.org/wiki/ISO_3166-1) format.
//!
//! It results in a mildly adapted [IETF language tag](https://en.wikipedia.org/wiki/IETF_language_tag).

use iso_country::Country;
use isolang::Language;

use std::{fmt, str::FromStr};

use serde::de::{self, Deserialize, Deserializer};
use serde::ser::Serializer;

/// 5 digit language and country code as used by the dictionaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lang5 {
    pub lang: Language,
    pub country: Country,
}

impl PartialEq<str> for Lang5 {
    fn eq(&self, other: &str) -> bool {
        self.to_string().as_str() == other
    }
}

impl<X> PartialEq<X> for Lang5
where
    X: AsRef<str>,
{
    fn eq(&self, other: &X) -> bool {
        self.to_string().as_str() == other.as_ref()
    }
}

impl<'a> PartialEq<Lang5> for &'a str {
    fn eq(&self, other: &Lang5) -> bool {
        let other = other.to_string();
        *self == other.as_str()
    }
}

impl PartialEq<Lang5> for String {
    fn eq(&self, other: &Lang5) -> bool {
        *self == other.to_string()
    }
}

impl Default for Lang5 {
    fn default() -> Self {
        Self::en_US
    }
}

impl Lang5 {
    #[allow(non_upper_case_globals)]
    pub const en_US: Lang5 = Lang5 {
        lang: Language::Eng,
        country: Country::US,
    };
}

impl fmt::Display for Lang5 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.lang.to_639_1().unwrap_or("??"))?;
        f.write_str("_")?;
        write!(f, "{}", self.country)?;
        Ok(())
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("Wrong character, expected '_' found '{0}'")]
struct Lang5SpacerError(char);

#[derive(Debug, Clone, Copy, Default)]
struct Lang5Visitor;

impl<'de> de::Visitor<'de> for Lang5Visitor {
    type Value = Lang5;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(
            formatter,
            "Expected a 5 digit lang and country code in the form of LL_CC"
        )
    }

    fn visit_borrowed_str<E>(self, s: &'de str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if s.len() != 5 {
            return Err(Lang5SpacerError('l')).map_err(serde::de::Error::custom);
        }
        let lang = Language::from_639_1(&s[0..2])
            .ok_or_else(|| Lang5SpacerError('2'))
            .map_err(serde::de::Error::custom)?;
        let c = s.chars().nth(2).unwrap();
        if c != '_' {
            return Err(Lang5SpacerError(c)).map_err(serde::de::Error::custom)?;
        }
        let country = Country::from_str(&s[3..5]).map_err(serde::de::Error::custom)?;
        Ok(Lang5 { lang, country })
    }
}

impl<'de> Deserialize<'de> for Lang5 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(Lang5Visitor)
    }
}

impl serde::Serialize for Lang5 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.to_string().as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;

    const EXPECTED: Lang5 = Lang5 {
        lang: Language::Deu,
        country: Country::AU,
    };
    const S: &str = "de_AU";

    #[test]
    fn iso_lang_german_austria_serde() {
        assert_eq!(S.to_owned(), EXPECTED.to_string());

        assert_matches!(serde_plain::from_str::<Lang5>(S), Ok(x) => assert_eq!(EXPECTED, x));
    }

    #[test]
    fn cmp_variants() {
        assert!(EXPECTED == S);
        assert!(EXPECTED == &S);
        assert!(EXPECTED == S.to_owned());
        assert!(EXPECTED == &S.to_owned());
        assert!(&EXPECTED == S);
    }
}
