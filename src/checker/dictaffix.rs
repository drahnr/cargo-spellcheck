use super::hunspell::cache_builtin;
use super::Result;
use crate::config::{Lang5, SearchDirs};
use color_eyre::eyre::{bail, eyre, WrapErr};
use fs_err as fs;
use itertools::Itertools;
use std::io;
use std::io::BufRead;
use std::path::{Path, PathBuf};

pub(crate) struct DicAff {
    pub(crate) dic: String,
    pub(crate) aff: String,
}

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
        .into_iter()
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

        let dic = fs_err::read_to_string(&dic)?;
        let aff = fs_err::read_to_string(&aff)?;

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
            let extra_dic = fs::read_to_string(extra_dic_path)?;
            is_valid_hunspell_dic(&mut extra_dic.as_bytes())?;
            log::trace!(target: "affdic", "Adding extra dict to main dict: {}", extra_dic.trim().lines().count() - 1);
            dic_acc.push('\n');
            // trim the initil number
            dic_acc.push_str(
                extra_dic
                    .trim()
                    .split_once("\n")
                    .expect("It's a valid dictionary. qed")
                    .1,
            );
        }

        // sort them, just in case
        let mut counter = 0;
        let dic = dic_acc
            .lines()
            .inspect(|_line| counter += 1)
            .into_iter()
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
pub(crate) fn is_valid_hunspell_dic_path(path: impl AsRef<Path>) -> Result<()> {
    let reader = io::BufReader::new(fs::File::open(path.as_ref())?);
    is_valid_hunspell_dic(reader)
}

/// Check a reader for correct hunspell format.
pub(crate) fn is_valid_hunspell_dic(reader: impl BufRead) -> Result<()> {
    let mut iter = reader.lines().enumerate();
    if let Some((_lineno, first)) = iter.next() {
        let first = first?;
        let _ = first.parse::<u64>().wrap_err_with(|| {
            eyre!("First line of extra dictionary must a number, but is: >{first}<")
        })?;
    }
    // Just check the first 10 lines, don't waste much time here
    // the first two are the most important ones.
    for (lineno, line) in iter.take(10) {
        // All lines after must be format x.
        if let Ok(num) = line?.parse::<i64>() {
            bail!("Line {lineno} of extra dictionary must not be a number, but is: >{num}<",)
        };
    }
    Ok(())
}
