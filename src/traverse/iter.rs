use super::*;
use crate::Documentation;

use fs_err as fs;

use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

/// An iterator traversing module hierarchies yielding paths
#[derive(Debug, Clone)]
pub struct TraverseModulesIter {
    /// state for enqueuing child files and the depth at which they are found
    queue: VecDeque<(PathBuf, usize)>,
    /// zero limits to the provided path, if it is a directory, all children are
    /// collected
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
        let path = fs::canonicalize(path)?;
        let meta = fs::metadata(&path)?;
        if meta.is_file() {
            self.queue.push_front((path, level));
        } else if meta.is_dir() {
            ignore::WalkBuilder::new(path)
                .git_ignore(true)
                .max_depth(1.into())
                .same_file_system(true)
                .skip_stdout(true)
                .build()
                .filter_map(|entry| {
                    entry
                        .ok()
                        .filter(|entry| entry.file_type().map(|ft| ft.is_file()).unwrap_or(false))
                        .map(|x| x.path().to_owned())
                })
                .filter(|path: &PathBuf| {
                    path.to_str()
                        .map(|x| x.to_owned())
                        .filter(|path| path.ends_with(".rs"))
                        .is_some()
                })
                .try_for_each::<_, Result<()>>(|path| {
                    log::trace!("🌱 using path {} as seed recursion dir", path.display());
                    self.queue.push_front((path, level));
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
            log::trace!("🥞 collecting mods declared in file {}", path.display());
            self.queue.extend(
                extract_modules_from_file(path)?
                    .into_iter()
                    .map(|item| (item, level)),
            );
        } else {
            log::warn!("🥞 Only dealing with files, dropping {}", path.display());
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
                // TODO potentially consider returning a result covering this
                let _ = self.collect_modules(path.as_path(), level + 1);
            }
            Some(path)
        } else {
            None
        }
    }
}

/// traverse path with a depth limit, if the path is a directory all its
/// children will be collected instead
// TODO should not read the documentation, that is out of scope.
// TODO should not have knowledge of `dev_comments`.
pub(crate) fn traverse(
    path: &Path,
    doc_comments: bool,
    dev_comments: bool,
    output_paths: crate::paths::OutputPaths,
) -> Result<impl Iterator<Item = Documentation>> {
    traverse_with_depth_limit(path, usize::MAX, doc_comments, dev_comments, output_paths)
}

/// traverse path with a depth limit, if the path is a directory all its
/// children will be collected as depth 0 instead
pub(crate) fn traverse_with_depth_limit(
    path: &Path,
    max_depth: usize,
    doc_comments: bool,
    dev_comments: bool,
    output_paths: crate::paths::OutputPaths,
) -> Result<impl Iterator<Item = Documentation>> {
    let it = TraverseModulesIter::with_depth_limit(path, max_depth)?
        .filter_map(move |path: PathBuf| -> Option<Documentation> {
            fs::read_to_string(&path).ok().map(|content| {
                let mut documentation = Documentation::new();
                documentation.set_warning_path(output_paths.path(&path));
                let origin = ContentOrigin::RustSourceFile(path);
                if let Err(error) = documentation.add_rust(
                    origin.clone(), content.as_str(), doc_comments, dev_comments,
                ) {
                    log::warn!("BUG: Failed to load content from {origin} (dev_comments={dev_comments:?}): {error:?}");
                }
                documentation
            })
        })
        .filter(|documentation| {
            !documentation.is_empty() || !documentation.warnings().is_empty()
        });
    Ok(it)
}
