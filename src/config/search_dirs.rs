use super::*;

#[derive(Debug, Clone)]
pub struct SearchDirs(pub Option<Vec<PathBuf>>);

impl std::ops::Deref for SearchDirs {
    type Target = Option<Vec<PathBuf>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::convert::AsRef<Option<Vec<PathBuf>>> for SearchDirs {
    fn as_ref(&self) -> &Option<Vec<PathBuf>> {
        &self.0
    }
}

impl Serialize for SearchDirs {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        if let Some(x) = self.as_ref() {
            serializer.serialize_some(x)
        } else {
            serializer.serialize_none()
        }
    }
}

impl<'de> Deserialize<'de> for SearchDirs {
    fn deserialize<D>(deserializer: D) -> Result<SearchDirs, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        deserializer
            .deserialize_option(SearchDirVisitor)
            .map(Into::into)
    }
}

impl Into<Option<Vec<PathBuf>>> for SearchDirs {
    fn into(self) -> Option<Vec<PathBuf>> {
        self.0
    }
}

impl From<Option<Vec<PathBuf>>> for SearchDirs {
    fn from(other: Option<Vec<PathBuf>>) -> SearchDirs {
        SearchDirs(other)
    }
}

struct SearchDirVisitor;

impl<'de> serde::de::Visitor<'de> for SearchDirVisitor {
    type Value = Option<Vec<PathBuf>>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("Search Dir Visitors must be an optional sequence of path")
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        Ok(deserializer.deserialize_seq(self)?.map(|mut seq| {
            seq.extend(
                os_specific_search_dirs()
                    .iter()
                    .map(|path: &PathBuf| PathBuf::from(path)),
            );
            seq
        }))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Some(os_specific_search_dirs().to_vec()))
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let mut v = Vec::with_capacity(8);
        while let Some(item) = seq.next_element()? {
            v.push(item);
        }
        Ok(Some(v))
    }
}
