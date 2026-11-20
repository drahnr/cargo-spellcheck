//! Lexical path formatting without filesystem access or symlink resolution.

use std::path::{Component, Path, PathBuf};

#[cfg(test)]
mod tests;

/// Express an absolute `path` relative to an absolute directory `base`.
///
/// Shared components are removed and `..` components lead back from `base` to
/// the common ancestor. Equal paths produce `.`. Relative inputs, relative bases,
/// and incompatible roots or prefixes leave `path` unchanged. Dot and parent
/// components are normalized lexically, without resolving symbolic links.
pub fn relative_path(path: &Path, base: &Path) -> PathBuf {
    if !path.is_absolute() || !base.is_absolute() {
        return path.to_path_buf();
    }

    let path_components = absolute_components(path);
    let base_components = absolute_components(base);
    let shared = path_components
        .iter()
        .zip(&base_components)
        .take_while(|(path_component, base_component)| path_component == base_component)
        .count();
    let path_suffix = &path_components[shared..];
    let base_suffix = &base_components[shared..];
    if path_suffix
        .iter()
        .chain(base_suffix)
        .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
    {
        return path.to_path_buf();
    }

    let result = PathBuf::from_iter(
        std::iter::repeat(Component::ParentDir.as_os_str())
            .take(base_suffix.len())
            .chain(path_suffix.iter().copied().map(Component::as_os_str)),
    );
    if result.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        result
    }
}

/// Normalize absolute path components, keeping parents from escaping the root.
fn absolute_components(path: &Path) -> Vec<Component<'_>> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(components.last(), Some(Component::Normal(_))) {
                    components.pop();
                }
            }
            _ => components.push(component),
        }
    }
    components
}
