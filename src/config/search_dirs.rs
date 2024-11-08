use super::*;

/// Obtain OS specific search directories.
fn os_specific_search_dirs() -> &'static [PathBuf] {
    lazy_static::lazy_static! {
        static ref OS_SPECIFIC_LOOKUP_DIRS: Vec<PathBuf> =
            if cfg!(target_os = "macos") {
                directories::BaseDirs::new()
                    .map(|base| vec![base.home_dir().to_owned().join("/Library/Spelling/"), PathBuf::from("/Library/Spelling/")])
                    .unwrap_or_default()
            } else if cfg!(target_os = "linux") {
                vec![
                    // Fedora
                    PathBuf::from("/usr/share/myspell/"),
                    PathBuf::from("/usr/share/hunspell/"),
                    // Arch Linux
                    PathBuf::from("/usr/share/myspell/dicts/"),
                ]
            } else {
                Vec::new()
            };

    }
    OS_SPECIFIC_LOOKUP_DIRS.as_slice()
}

/// A collection of search directories. OS specific paths are only provided in
/// the iterator.
#[derive(Debug, Clone)]
pub struct SearchDirs(pub Vec<PathBuf>);

impl Default for SearchDirs {
    fn default() -> Self {
        Self(Vec::with_capacity(8))
    }
}

impl SearchDirs {
    pub fn iter(&self, extend_by_os: bool) -> impl Iterator<Item = &PathBuf> {
        let chained = if extend_by_os {
            os_specific_search_dirs().iter()
        } else {
            [].iter()
        };
        self.0.iter().chain(chained)
    }
}

impl std::convert::AsRef<Vec<PathBuf>> for SearchDirs {
    fn as_ref(&self) -> &Vec<PathBuf> {
        &self.0
    }
}

impl Serialize for SearchDirs {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        serializer.serialize_newtype_struct("SearchDirs", &self.0)
    }
}

impl<'de> Deserialize<'de> for SearchDirs {
    fn deserialize<D>(deserializer: D) -> Result<SearchDirs, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        deserializer
            .deserialize_newtype_struct("SearchDirs", SearchDirVisitor)
            .map(Into::into)
    }
}

impl From<SearchDirs> for Vec<PathBuf> {
    fn from(val: SearchDirs) -> Self {
        val.0
    }
}

impl From<Vec<PathBuf>> for SearchDirs {
    fn from(other: Vec<PathBuf>) -> SearchDirs {
        SearchDirs(other)
    }
}

/// A search directory visitor, auto extending the search directory with OS
/// defaults.
struct SearchDirVisitor;

impl<'de> serde::de::Visitor<'de> for SearchDirVisitor {
    type Value = Vec<PathBuf>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("Search Dir Visitors must be an optional sequence of path")
    }

    fn visit_newtype_struct<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let seq = deserializer.deserialize_seq(self)?;
        Ok(seq)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let mut v = Vec::with_capacity(8);
        while let Some(item) = seq.next_element()? {
            v.push(item);
        }
        Ok(v)
    }
}
