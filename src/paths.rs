//! Output path selection, independent of paths used to read and modify files.

use crate::errors::Result;
use doc_chunks::paths::relative_path;
use std::path::{Path, PathBuf};

/// Directories supplied by the caller, or the working directory when omitted.
#[derive(Debug, Clone)]
pub(crate) struct OutputPaths {
    bases: Vec<PathBuf>,
    full_paths: bool,
}

impl OutputPaths {
    /// Resolve display bases without changing the process working directory.
    pub(crate) fn new(paths: &[PathBuf], full_paths: bool) -> Result<Self> {
        let cwd = crate::traverse::cwd()?.canonicalize()?;
        let mut bases = Vec::from_iter(paths.iter().filter_map(|path| {
            let path = cwd.join(path).canonicalize().ok()?;
            if path.is_dir() {
                Some(path)
            } else {
                path.parent().map(Path::to_path_buf)
            }
        }));
        if bases.is_empty() {
            bases.push(cwd);
        }
        Ok(Self { bases, full_paths })
    }

    /// Choose the closest supplied directory, retaining the first for external files.
    pub(crate) fn base_for(&self, path: &Path) -> Option<&Path> {
        if self.full_paths {
            return None;
        }
        self.bases
            .iter()
            .filter(|base| path.starts_with(base))
            .max_by_key(|base| base.components().count())
            .or_else(|| self.bases.first())
            .map(PathBuf::as_path)
    }

    /// Format a source path while leaving its filesystem identity unchanged.
    pub(crate) fn path(&self, path: &Path) -> PathBuf {
        match self.base_for(path) {
            Some(base) => relative_path(path, base),
            None => path.to_path_buf(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Default output paths are relative to the working directory.
    #[test]
    fn defaults_to_cwd() {
        let inputs = Vec::new();
        let paths = OutputPaths::new(&inputs, false).unwrap();
        let input = crate::traverse::cwd()
            .unwrap()
            .canonicalize()
            .unwrap()
            .join("src/main.rs");
        let expected = PathBuf::from("src/main.rs");
        assert_eq!(paths.path(&input), expected);
    }

    /// Directory and file arguments establish the directory used for display.
    #[test]
    fn explicit_bases_and_full_paths() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .unwrap();
        let input = root.join("doc-chunks/src/lib.rs");
        // Each input list is paired with its expected displayed source path.
        let cases = [
            (vec![root.join("doc-chunks")], PathBuf::from("src/lib.rs")),
            (
                vec![root.join("doc-chunks/Cargo.toml")],
                PathBuf::from("src/lib.rs"),
            ),
            (vec![input.clone()], PathBuf::from("lib.rs")),
            (
                vec![root.clone(), root.join("doc-chunks")],
                PathBuf::from("src/lib.rs"),
            ),
        ];
        for (inputs, expected) in cases {
            let paths = OutputPaths::new(&inputs, false).unwrap();
            assert_eq!(paths.path(&input), expected);
            let full_paths = OutputPaths::new(&inputs, true).unwrap();
            let expected_full = input.clone();
            assert_eq!(full_paths.path(&input), expected_full);
        }
    }
}
