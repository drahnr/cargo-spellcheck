//! Shared Hunspell-format dictionary loading.

#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
use super::cached::cache_builtin;
use super::Result;
#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
use crate::config::{Lang5, SearchDirs};
use color_eyre::eyre::{bail, eyre, WrapErr};
use fs_err as fs;
#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
use itertools::Itertools;
use std::io::BufRead;
#[cfg(feature = "hunspell")]
use std::path::Path;
#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
use std::path::PathBuf;

#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
pub(crate) struct DicAff {
    pub(crate) dic: String,
    pub(crate) aff: String,
}

#[cfg(any(feature = "hunspell", feature = "zet", feature = "spellbook"))]
impl DicAff {
    pub(crate) fn load(
        extra_dictionaries: &[std::path::PathBuf],
        search_dirs: &SearchDirs,
        lang: Lang5,
        use_builtin: bool,
        skip_os_lookups: bool,
    ) -> Result<Self> {
        let lang = lang.to_string();
        let lang = lang.as_str();

        // lookup paths are really just an attempt to provide a dictionary, so be more forgiving
        // when encountering errors here
        let (dic, aff): (PathBuf, PathBuf) = search_dirs.iter(!skip_os_lookups)
        .filter(|search_dir| {
            let keep = search_dir.is_dir();
            if !keep {
                // search_dir also contains the default paths, so just silently ignore these
                log::debug!(
                    target: "affdic",
                    "Dictionary search path is not a directory {}",
                    search_dir.display()
                );
            } else {
                log::debug!(
                    target: "affdic",
                    "Found dictionary search path {}",
                    search_dir.display()
                );
            }
            keep
        })
        .find_map(|search_dir| {
            let dic = search_dir.join(lang).with_extension("dic");
            if !dic.is_file() {
                log::debug!(
                    target: "affdic",
                    "Dictionary path dervied from search dir is not a file {}",
                    dic.display()
                );
                return None;
            }
            let aff = search_dir.join(lang).with_extension("aff");
            if !aff.is_file() {
                log::debug!(
                    target: "affdic",
                    "Affixes path dervied from search dir is not a file {}",
                    aff.display()
                );
                return None;
            }
            log::debug!("Using dic {} and aff {}", dic.display(), aff.display());
            Some((dic, aff))
        })
        .ok_or_else(|| {
            eyre!("Failed to find any {lang}.dic / {lang}.aff in any search dir or no search provided",
                lang = lang)
        })
        .or_else(|e| {
            if use_builtin {
                Ok(cache_builtin()?)
            } else {
                Err(e)
            }
        })?;

        let dic = String::from_utf8(fs::read(&dic)?).wrap_err_with(|| {
            format!(
                "Unsupported dictionary encoding in {}: expected UTF-8 text",
                dic.display()
            )
        })?;
        let aff = String::from_utf8(fs::read(&aff)?).wrap_err_with(|| {
            format!(
                "Unsupported affix encoding in {}: expected UTF-8 text",
                aff.display()
            )
        })?;

        // We need to combine multiple dictionaries into one
        // since we want suffix support rather than plain word lists
        let mut dic_acc = dic;

        // suggestion must contain the word itself if it is valid extra dictionary
        // be more strict about the extra dictionaries, they have to exist
        log::info!(target: "dicaff", "Adding {} extra dictionaries", extra_dictionaries.len());

        for extra_dic_path in extra_dictionaries {
            log::debug!(target: "affdic", "Adding extra dictionary {}", extra_dic_path.display());
            // after calling `sanitize_paths`
            // the ought to be all absolutes
            assert!(extra_dic_path.is_absolute());
            let extra_dic = String::from_utf8(fs::read(extra_dic_path)?).wrap_err_with(|| {
                format!(
                    "Unsupported dictionary encoding in {}: expected UTF-8 text",
                    extra_dic_path.display()
                )
            })?;
            is_valid_hunspell_dic(&mut extra_dic.as_bytes())?;
            // A valid header-only dictionary (including "0\n") has no entries.
            let body = extra_dic
                .split_once('\n')
                .map_or("", |(_, body)| body)
                .trim();
            log::trace!(target: "affdic", "Adding extra dict to main dict: {}", body.lines().count());
            if !body.is_empty() {
                dic_acc.push('\n');
                dic_acc.push_str(body);
            }
        }

        // sort them, just in case
        let mut counter = 0;
        let dic = dic_acc
            .lines()
            .inspect(|_line| counter += 1)
            .sorted()
            .unique()
            .join("\n");
        let counter = counter.to_string();
        let dic = counter + "\n" + dic.trim();

        log::trace!(target: "affdic", "Total dictionary entries are: {}", dic.trim().lines().count() - 1);

        Ok(Self { dic, aff })
    }
}

/// Check if provided path has valid dictionary format.
///
/// This is a YOLO check.
#[cfg(feature = "hunspell")]
pub(crate) fn is_valid_hunspell_dic_path(path: impl AsRef<Path>) -> Result<()> {
    let reader = std::io::BufReader::new(fs::File::open(path.as_ref())?);
    is_valid_hunspell_dic(reader)
}

/// Check a reader for correct hunspell format.
pub(crate) fn is_valid_hunspell_dic(reader: impl BufRead) -> Result<()> {
    let mut iter = reader.lines().enumerate();
    let (_, first) = iter
        .next()
        .ok_or_else(|| eyre!("First line of extra dictionary must be a number, but is missing"))?;
    let first = first?;
    let _ = first.parse::<u64>().wrap_err_with(|| {
        eyre!("First line of extra dictionary must a number, but is: >{first}<")
    })?;
    // Just check the first 10 lines, don't waste much time here
    // the first two are the most important ones.
    for (lineno, line) in iter.take(10) {
        // All lines after must be format x.
        if let Ok(num) = line?.parse::<i64>() {
            bail!("Line {lineno} of extra dictionary must not be a number, but is: >{num}<",);
        };
    }
    Ok(())
}

#[cfg(all(test, any(feature = "zet", feature = "spellbook")))]
mod tests {
    use super::super::cached::{cache_builtin_inner, BUILTIN_HUNSPELL_DIC};
    use super::*;

    struct DictionaryFixture(PathBuf);

    impl DictionaryFixture {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "cargo-spellcheck-dictaffix-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("en_US.aff"), "SET UTF-8\n").unwrap();
            fs::write(directory.join("en_US.dic"), "1\nFixtureword\n").unwrap();
            Self(directory)
        }

        fn load(&self, extras: &[PathBuf]) -> Result<DicAff> {
            DicAff::load(
                extras,
                &SearchDirs::from(vec![self.0.clone()]),
                Lang5::en_US,
                false,
                true,
            )
        }
    }

    impl Drop for DictionaryFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn empty_extra_dictionary_leaves_base_dictionary_unchanged() {
        let fixture = DictionaryFixture::new();
        let baseline = fixture.load(&[]).unwrap();
        let extra = fixture.0.join("extra.dic");
        for contents in ["0\n", "0", "0\r\n", "0\n\n", "0\n \t\n"] {
            fs::write(&extra, contents).unwrap();
            let loaded = fixture.load(std::slice::from_ref(&extra)).unwrap();
            assert_eq!(loaded.dic, baseline.dic, "{contents:?}");
            assert_eq!(loaded.aff, baseline.aff, "{contents:?}");
        }
    }

    #[test]
    fn extra_dictionary_with_invalid_or_missing_header_is_rejected() {
        let fixture = DictionaryFixture::new();
        let extra = fixture.0.join("extra.dic");
        for contents in ["", "\n", "invalid\n", "invalid\nFixtureword\n"] {
            fs::write(&extra, contents).unwrap();
            let error = fixture
                .load(std::slice::from_ref(&extra))
                .err()
                .expect("invalid header must fail validation");
            assert!(error.to_string().contains("First line"), "{error:?}");
            assert_eq!(fs::read_to_string(&extra).unwrap(), contents);
        }
    }

    #[test]
    fn extra_dictionary_body_is_still_loaded() {
        let fixture = DictionaryFixture::new();
        let extra = fixture.0.join("extra.dic");
        for contents in ["1\nExtraentry\n", "1\nExtraentry", "1\r\nExtraentry\r\n"] {
            fs::write(&extra, contents).unwrap();
            let loaded = fixture.load(std::slice::from_ref(&extra)).unwrap();
            assert!(loaded.dic.lines().any(|line| line == "Extraentry"));
            assert!(loaded.dic.lines().any(|line| line == "Fixtureword"));
        }
    }

    #[test]
    fn builtin_cache_writes_dictionary_bytes_and_reuses_existing_file() {
        let fixture = DictionaryFixture::new();
        let path = cache_builtin_inner(&fixture.0, "dic", BUILTIN_HUNSPELL_DIC).unwrap();
        assert_eq!(fs::read(&path).unwrap(), BUILTIN_HUNSPELL_DIC);
        let reused = cache_builtin_inner(&fixture.0, "dic", BUILTIN_HUNSPELL_DIC).unwrap();
        assert_eq!(reused, path);
        assert_eq!(fs::read(&reused).unwrap(), BUILTIN_HUNSPELL_DIC);
    }
}
