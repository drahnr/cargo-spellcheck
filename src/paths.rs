//! Display-only path formatting inferred from the original CLI inputs.

use std::path::{Path, PathBuf};

/// Formats output paths without changing source paths or the working directory.
pub(crate) struct OutputPaths {
    bases: Vec<Base>,
    absolute_paths: bool,
}

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

#[cfg(test)]
mod tests {
    use super::{Base, OutputPaths};
    use std::fs;
    use std::path::{Path, PathBuf};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            Self::in_dir(&std::env::temp_dir())
        }

        fn in_dir(parent: &Path) -> Self {
            let path = parent.join(format!("spellcheck-paths-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }

        fn file(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "").unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // Relative CLI roots can be modeled without changing the process-wide cwd.
    fn prefixed(bases: &[(&Path, &str)]) -> OutputPaths {
        OutputPaths {
            bases: bases
                .iter()
                .map(|(root, prefix)| Base {
                    root: root.canonicalize().unwrap(),
                    prefix: PathBuf::from(prefix),
                })
                .collect(),
            absolute_paths: false,
        }
    }

    #[test]
    fn no_inputs_use_cwd_and_leave_outside_paths_unchanged() {
        let cwd = std::env::current_dir().unwrap();
        let local = TempDir::in_dir(&cwd);
        let source = local.file("src/lib.rs");
        let paths = OutputPaths::new(&[], false).unwrap();
        assert_eq!(
            paths.path(&source),
            source.strip_prefix(cwd.canonicalize().unwrap()).unwrap()
        );
        let relative = source.strip_prefix(cwd.canonicalize().unwrap()).unwrap();
        assert_eq!(paths.path(relative), relative);

        let outside = TempDir::new();
        let source = outside.file("outside.rs");
        if !source.starts_with(cwd.canonicalize().unwrap()) {
            assert_eq!(paths.path(&source), source);
        }
    }

    #[test]
    fn single_directory_uses_itself() {
        let temp = TempDir::new();
        let source = temp.file("src/lib.rs");
        let paths = OutputPaths::new(&[temp.0.clone()], false).unwrap();
        assert_eq!(paths.path(&source), Path::new("src/lib.rs"));
    }

    #[test]
    fn single_file_uses_its_parent() {
        let temp = TempDir::new();
        let source = temp.file("src/lib.rs");
        let sibling = temp.file("src/other.rs");
        let paths = OutputPaths::new(&[source.clone()], false).unwrap();
        assert_eq!(paths.path(&source), Path::new("lib.rs"));
        assert_eq!(paths.path(&sibling), Path::new("other.rs"));
        let outside = temp.file("outside.rs");
        assert_eq!(paths.path(&outside), outside);
    }

    #[test]
    fn multiple_relative_directories_preserve_original_prefixes() {
        let temp = TempDir::new();
        let a = temp.file("a/src/lib.rs");
        let b = temp.file("b/src/lib.rs");
        let paths = prefixed(&[(&temp.0.join("a"), "a"), (&temp.0.join("b"), "b")]);
        assert_eq!(paths.path(&a), Path::new("a/src/lib.rs"));
        assert_eq!(paths.path(&b), Path::new("b/src/lib.rs"));
    }

    #[test]
    fn multiple_relative_files_preserve_original_prefixes() {
        let temp = TempDir::new();
        let a = temp.file("a/src/lib.rs");
        let b = temp.file("b/src/lib.rs");
        let paths = prefixed(&[
            (a.parent().unwrap(), "a/src"),
            (b.parent().unwrap(), "b/src"),
        ]);
        assert_eq!(paths.path(&a), Path::new("a/src/lib.rs"));
        assert_eq!(paths.path(&b), Path::new("b/src/lib.rs"));
    }

    #[test]
    fn multiple_absolute_inputs_remain_absolute() {
        let temp = TempDir::new();
        let a = temp.file("a/src/lib.rs");
        let b = temp.file("b/src/lib.rs");
        for inputs in [
            vec![temp.0.join("a"), temp.0.join("b")],
            vec![a.clone(), b.clone()],
        ] {
            let paths = OutputPaths::new(&inputs, false).unwrap();
            assert_eq!(paths.path(&a), a);
            assert_eq!(paths.path(&b), b);
        }
    }

    #[test]
    fn nested_roots_choose_the_closest_in_either_order() {
        let temp = TempDir::new();
        let source = temp.file("nested/src/lib.rs");
        let nested = temp.0.join("nested");
        for bases in [
            vec![(temp.0.as_path(), "outer"), (nested.as_path(), "inner")],
            vec![(nested.as_path(), "inner"), (temp.0.as_path(), "outer")],
        ] {
            assert_eq!(
                prefixed(&bases).path(&source),
                Path::new("inner/src/lib.rs")
            );
        }
    }

    #[test]
    fn duplicate_roots_use_the_first_original_prefix() {
        let temp = TempDir::new();
        let source = temp.file("src/lib.rs");
        let paths = prefixed(&[(&temp.0, "first"), (&temp.0, "second")]);
        assert_eq!(paths.path(&source), Path::new("first/src/lib.rs"));
        let paths = OutputPaths::new(&[temp.0.clone(), temp.0.clone()], false).unwrap();
        assert_eq!(paths.path(&source), source);
    }

    #[test]
    fn unresolved_sources_inputs_and_outside_sources_fall_back() {
        let temp = TempDir::new();
        let source = temp.file("src/lib.rs");
        let missing = temp.0.join("missing.rs");
        let paths = OutputPaths::new(&[temp.0.join("src")], false).unwrap();
        assert_eq!(paths.path(&missing), missing);
        let outside = temp.file("outside.rs");
        assert_eq!(paths.path(&outside), outside);
        let paths = OutputPaths::new(&[temp.0.join("missing")], false).unwrap();
        assert_eq!(paths.path(&source), source);
    }

    #[test]
    fn absolute_paths_preserve_unresolved_spelling() {
        let temp = TempDir::new();
        let source = temp.file("src/lib.rs");
        let paths = OutputPaths::new(&[temp.0.clone()], true).unwrap();
        assert_eq!(paths.path(&source), source);
        let unresolved = Path::new("missing/../original.rs");
        assert_eq!(paths.path(unresolved), unresolved);
        assert_eq!(OutputPaths::new(&[], true).unwrap().path(&source), source);
    }

    #[test]
    fn longer_prefixes_do_not_lengthen_output() {
        let temp = TempDir::new();
        let source = temp.file("lib.rs");
        let prefix = "very-long-original-prefix/".repeat(source.as_os_str().len());
        let paths = prefixed(&[(&temp.0, &prefix)]);
        assert_eq!(paths.path(&source), source);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_parent_components_are_resolved_before_shortening() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new();
        let source = temp.file("actual/lib.rs");
        fs::create_dir(temp.0.join("actual/child")).unwrap();
        symlink(temp.0.join("actual/child"), temp.0.join("link")).unwrap();
        let original = temp.0.join("link/../lib.rs");
        assert_eq!(original.canonicalize().unwrap(), source);
        let paths = OutputPaths::new(&[temp.0.clone()], false).unwrap();
        assert_eq!(paths.path(&original), Path::new("actual/lib.rs"));
        let paths = OutputPaths::new(&[temp.0.join("link/..")], false).unwrap();
        assert_eq!(paths.path(&original), Path::new("lib.rs"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_directory_prefix_keeps_the_original_cli_identity() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new();
        let source = temp.file("actual/src/lib.rs");
        symlink(temp.0.join("actual"), temp.0.join("alias")).unwrap();
        let paths = prefixed(&[(&temp.0.join("alias"), "alias")]);
        assert_eq!(paths.path(&source), Path::new("alias/src/lib.rs"));
        let paths = OutputPaths::new(&[temp.0.join("alias")], false).unwrap();
        assert_eq!(paths.path(&source), Path::new("src/lib.rs"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_file_uses_original_parent_not_target_parent() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new();
        let target = temp.file("target/real.rs");
        fs::create_dir(temp.0.join("input")).unwrap();
        let input = temp.0.join("input/alias.rs");
        symlink(&target, &input).unwrap();
        for inputs in [vec![input.clone()], vec![input.clone(), input.clone()]] {
            let paths = OutputPaths::new(&inputs, false).unwrap();
            assert_eq!(paths.bases[0].root, input.parent().unwrap());
            assert_eq!(paths.path(&target), target);
            assert_eq!(paths.path(&input), input);
        }
    }
}
