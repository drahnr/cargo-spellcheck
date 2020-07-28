use super::*;
use crate::Documentation;

use std::fs;

use log::{trace, warn};

use std::path::{Path, PathBuf};

use anyhow::Result;

/// An iterator traversing module hierarchies yielding paths
#[derive(Debug, Clone)]
pub struct TraverseModulesIter {
    /// state for enqueuing child files and the depth at which they are found
    queue: VecDeque<(PathBuf, usize)>,
    /// zero limits to the provided path, if it is a directory, all children are collected
    max_depth: usize,
}

impl Default for TraverseModulesIter {
    fn default() -> Self {
        Self {
            max_depth: usize::MAX,
            queue: VecDeque::with_capacity(128),
        }
    }
}

impl TraverseModulesIter {
    fn add_initial_path<P>(&mut self, path: P, level: usize) -> Result<()>
    where
        P: AsRef<Path>,
    {
        let path = path.as_ref();
        let path = path.canonicalize().map_err(|e| {
            anyhow!("Failed to canonicalize path {}", path.display()).context(e)
        })?;
        let meta = path.metadata().map_err(|e| {
            anyhow!("Failed to obtain meta data for path {}", path.display()).context(e)
        })?;
        if meta.is_file() {
            self.queue.push_back((path, level));
        } else if meta.is_dir() {
            walkdir::WalkDir::new(path)
                .max_depth(1)
                .same_file_system(true)
                .into_iter()
                .filter_map(|entry| {
                    entry
                        .ok()
                        .filter(|entry| entry.file_type().is_file())
                        .map(|x| x.path().to_owned())
                })
                .filter(|path: &PathBuf| {
                    path.to_str()
                        .map(|x| x.to_owned())
                        .filter(|path| path.ends_with(".rs"))
                        .is_some()
                })
                .try_for_each::<_, Result<()>>(|path| {
                    self.queue.push_back((path, level));
                    Ok(())
                })?;
        }
        Ok(())
    }

    #[allow(unused)]
    pub fn with_multi<P, J, I>(entries: I) -> Result<Self>
    where
        P: AsRef<Path>,
        J: Iterator<Item = P>,
        I: IntoIterator<Item = P, IntoIter = J>,
    {
        let mut me = Self::default();
        for path in entries.into_iter() {
            me.add_initial_path(path, 0)?;
        }
        Ok(me)
    }

    pub fn with_depth_limit<P: AsRef<Path>>(path: P, max_depth: usize) -> Result<Self> {
        let mut me = Self {
            max_depth,
            ..Default::default()
        };
        me.add_initial_path(path, 0)?;
        Ok(me)
    }

    /// Create a new path with (almost) infinite depth bounds
    #[allow(unused)]
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::with_depth_limit(path, usize::MAX)
    }

    pub fn collect_modules(&mut self, path: &Path, level: usize) -> Result<()> {
        if path.is_file() {
            trace!("collecting mods declared in file {}", path.display());
            self.queue.extend(
                extract_modules_from_file(path)?
                    .into_iter()
                    .map(|item| (item, level)),
            );
        } else {
            warn!("Only dealing with files, dropping {}", path.display());
        }
        Ok(())
    }
}

impl Iterator for TraverseModulesIter {
    type Item = PathBuf;
    fn next(&mut self) -> Option<Self::Item> {
        if let Some((path, level)) = self.queue.pop_front() {
            if level < self.max_depth {
                // ignore the error here, there is nothing we can do really
                // @todo potentially consider returning a result covering this
                let _ = self.collect_modules(path.as_path(), level + 1);
            }
            Some(path)
        } else {
            None
        }
    }
}

/// traverse path with a depth limit, if the path is a directory all its children will be collected
/// instead
pub(crate) fn traverse(path: &Path) -> Result<impl Iterator<Item = Documentation>> {
    traverse_with_depth_limit(path, usize::MAX)
}

/// traverse path with a depth limit, if the path is a directory all its children will be collected
/// as depth 0 instead
pub(crate) fn traverse_with_depth_limit(
    path: &Path,
    max_depth: usize,
) -> Result<impl Iterator<Item = Documentation>> {
    let it = TraverseModulesIter::with_depth_limit(path, max_depth)?
        .filter_map(|path: PathBuf| -> Option<Documentation> {
            fs::read_to_string(&path).ok().map(|content| {
                Documentation::from((ContentOrigin::RustSourceFile(path), content.as_str()))
            })
        })
        .filter(|documentation| !documentation.is_empty());
    Ok(it)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_dir() -> PathBuf {
        manifest_dir().join("demo")
    }

    #[test]
    fn traverse_main_rs() {
        let manifest_path = demo_dir().join("src/main.rs");

        let expect = indexmap::indexset! {
            "src/main.rs",
            "src/lib.rs",
            "src/nested/mod.rs",
            "src/nested/justone.rs",
            "src/nested/justtwo.rs",
            "src/nested/again/mod.rs",
            "src/nested/fragments.rs",
            "src/nested/fragments/enumerate.rs",
            "src/nested/fragments/simple.rs",
        }
        .into_iter()
        .map(|sub| demo_dir().join(sub))
        .collect::<indexmap::set::IndexSet<PathBuf>>();

        let found = TraverseModulesIter::new(manifest_path.as_path())
            .expect("Must succeed to traverse file tree.")
            .into_iter()
            .collect::<Vec<PathBuf>>();

        let unexpected_files: Vec<_> = dbg!(&found)
            .iter()
            .filter(|found_path| !expect.contains(*found_path))
            .collect();
        assert_eq!(Vec::<&PathBuf>::new(), unexpected_files);

        let missing_files: Vec<_> = expect
            .iter()
            .filter(|expected_path| !found.contains(expected_path))
            .collect();
        assert_eq!(Vec::<&PathBuf>::new(), missing_files);

        assert_eq!(found.len(), expect.len());
    }
}
