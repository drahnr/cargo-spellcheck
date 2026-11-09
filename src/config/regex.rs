use super::*;

#[derive(Debug)]
pub struct WrappedRegex(pub Regex);

impl Clone for WrappedRegex {
    fn clone(&self) -> Self {
        // @todo inefficient.. but right now this should almost never happen
        // @todo implement a lazy static `Arc<Mutex<HashMap<&'static str,Regex>>`
        Self(Regex::new(self.as_str()).unwrap())
    }
}

impl std::ops::Deref for WrappedRegex {
    type Target = Regex;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::convert::AsRef<Regex> for WrappedRegex {
    fn as_ref(&self) -> &Regex {
        &self.0
    }
}

impl Serialize for WrappedRegex {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WrappedRegex {
    fn deserialize<D>(deserializer: D) -> Result<WrappedRegex, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        deserializer
            .deserialize_any(RegexVisitor)
            .map(WrappedRegex::from)
    }
}

impl From<WrappedRegex> for Regex {
    fn from(val: WrappedRegex) -> Self {
        val.0
    }
}

impl From<Regex> for WrappedRegex {
    fn from(other: Regex) -> WrappedRegex {
        WrappedRegex(other)
    }
}

struct RegexVisitor;

impl<'de> serde::de::Visitor<'de> for RegexVisitor {
    type Value = Regex;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("String with valid regex expression")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        let re = Regex::new(value).map_err(E::custom)?;
        Ok(re)
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_str::<E>(value.as_str())
    }
}
