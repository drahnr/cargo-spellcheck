//! Display-only path formatting inferred from the original CLI inputs.

use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

/// Formats output paths without changing source paths or the working directory.
#[derive(Debug, Clone)]
pub(crate) struct OutputPaths {
    bases: Vec<Base>,
    absolute_paths: bool,
}

#[derive(Debug, Clone)]
struct Base {
    root: PathBuf,
    prefix: PathBuf,
}

impl OutputPaths {
    /// Uses the cwd, a single input's root, or the original multiple-input prefixes.
    pub(crate) fn new(inputs: &[PathBuf], absolute_paths: bool) -> crate::errors::Result<Self> {
        let mut paths = Self {
            bases: Vec::new(),
            absolute_paths,
        };
        if absolute_paths {
            return Ok(paths);
        }
        if inputs.is_empty() {
            paths.bases.push(Base {
                root: std::env::current_dir()?.canonicalize()?,
                prefix: PathBuf::new(),
            });
        } else {
            for input in inputs {
                // A symlink file's target parent need not identify the original input.
                let root = if input.is_file() {
                    input.parent().unwrap_or_else(|| Path::new("."))
                } else {
                    input.as_path()
                };
                let canonical = if root.as_os_str().is_empty() {
                    Path::new(".").canonicalize()
                } else {
                    root.canonicalize()
                };
                if let Ok(canonical) = canonical {
                    paths.bases.push(Base {
                        root: canonical,
                        prefix: if inputs.len() > 1 {
                            root.to_path_buf()
                        } else {
                            PathBuf::new()
                        },
                    });
                }
            }
        }
        Ok(paths)
    }

    /// Shortens a resolvable path only when its display form is actually shorter.
    pub(crate) fn path(&self, path: &Path) -> PathBuf {
        if self.absolute_paths {
            return path.to_path_buf();
        }
        if let Ok(canonical) = path.canonicalize() {
            // The closest root wins; equal roots keep the original argument order.
            let closest = self
                .bases
                .iter()
                .filter_map(|base| {
                    canonical
                        .strip_prefix(&base.root)
                        .ok()
                        .map(|suffix| (base, suffix))
                })
                .min_by_key(|(_, suffix)| suffix.components().count());
            if let Some((base, suffix)) = closest {
                let display = base.prefix.join(suffix);
                if !display.as_os_str().is_empty()
                    && display.as_os_str().len() < path.as_os_str().len()
                {
                    return display;
                }
            }
        }
        path.to_path_buf()
    }
}
