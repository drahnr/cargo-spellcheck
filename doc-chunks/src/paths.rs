//! Lexical path formatting without filesystem access or symlink resolution.

use std::path::{Component, Path, PathBuf};

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

#[cfg(test)]
mod tests {
    use super::*;

    /// A named input path and display base paired with the expected relative path.
    struct Case {
        name: &'static str,
        input: &'static str,
        base: &'static str,
        expected: &'static str,
    }

    /// Check lexical results and their display formatting against explicit expectations.
    fn check_cases(cases: &[Case]) {
        for Case {
            name,
            input,
            base,
            expected,
        } in cases
        {
            let input = Path::new(input);
            let base = Path::new(base);
            let expected_path = PathBuf::from(expected);
            let actual = relative_path(input, base);
            assert_eq!(actual, expected_path, "{name}");
            let actual_display = actual.display().to_string();
            let expected_display = *expected;
            assert_eq!(actual_display, expected_display, "{name}");
        }
    }

    /// Relative paths must retain their spelling even with redundant components.
    #[test]
    fn relative_inputs_are_unchanged() {
        let cases = [
            Case {
                name: "relative source",
                input: "src/transport.rs",
                base: "/workspace/delivery",
                expected: "src/transport.rs",
            },
            Case {
                name: "relative parents and dots",
                input: "../delivery/./socket.rs",
                base: "/workspace/delivery",
                expected: "../delivery/./socket.rs",
            },
            Case {
                name: "empty input",
                input: "",
                base: "/workspace/delivery",
                expected: "",
            },
        ];
        check_cases(&cases);
    }

    /// Absolute Unix paths support sibling, ancestor, and normalized lexical displays.
    #[cfg(unix)]
    #[test]
    fn absolute_paths_are_relative_to_the_base() {
        let cases = [
            Case {
                name: "child source",
                input: "/workspace/delivery/src/socket.rs",
                base: "/workspace/delivery",
                expected: "src/socket.rs",
            },
            Case {
                name: "sibling package",
                input: "/workspace/certificates/guide.md",
                base: "/workspace/delivery",
                expected: "../certificates/guide.md",
            },
            Case {
                name: "ancestor directory",
                input: "/workspace",
                base: "/workspace/delivery/src",
                expected: "../..",
            },
            Case {
                name: "equal paths",
                input: "/workspace/delivery",
                base: "/workspace/delivery",
                expected: ".",
            },
            Case {
                name: "root base",
                input: "/workspace/delivery",
                base: "/",
                expected: "workspace/delivery",
            },
            Case {
                name: "root target",
                input: "/",
                base: "/workspace/delivery",
                expected: "../..",
            },
            Case {
                name: "component boundaries",
                input: "/workspace/delivery-notes/guide.md",
                base: "/workspace/delivery",
                expected: "../delivery-notes/guide.md",
            },
            Case {
                name: "relative base",
                input: "/workspace/delivery/socket.rs",
                base: "delivery",
                expected: "/workspace/delivery/socket.rs",
            },
            Case {
                name: "normalized dot and parent components",
                input: "/workspace/./certificates/../delivery/socket.rs",
                base: "/workspace/delivery/src/..",
                expected: "socket.rs",
            },
            Case {
                name: "parents cannot escape the root",
                input: "/../../certificates/guide.md",
                base: "/workspace/delivery",
                expected: "../../certificates/guide.md",
            },
        ];
        check_cases(&cases);
    }

    /// Windows paths require matching drives and UNC roots before relativizing.
    #[cfg(windows)]
    #[test]
    fn windows_roots_and_prefixes_are_respected() {
        let cases = [
            Case {
                name: "child source",
                input: r"C:\workspace\delivery\src\socket.rs",
                base: r"C:\workspace\delivery",
                expected: r"src\socket.rs",
            },
            Case {
                name: "sibling package",
                input: r"C:\workspace\certificates\guide.md",
                base: r"C:\workspace\delivery",
                expected: r"..\certificates\guide.md",
            },
            Case {
                name: "ancestor directory",
                input: r"C:\workspace",
                base: r"C:\workspace\delivery\src",
                expected: r"..\..",
            },
            Case {
                name: "equal paths",
                input: r"C:\workspace\delivery",
                base: r"C:\workspace\delivery",
                expected: ".",
            },
            Case {
                name: "different drives",
                input: r"D:\certificates\guide.md",
                base: r"C:\workspace\delivery",
                expected: r"D:\certificates\guide.md",
            },
            Case {
                name: "different UNC shares",
                input: r"\\archive\certificates\guide.md",
                base: r"\\archive\delivery",
                expected: r"\\archive\certificates\guide.md",
            },
            Case {
                name: "drive-relative input",
                input: r"C:delivery\socket.rs",
                base: r"C:\workspace",
                expected: r"C:delivery\socket.rs",
            },
            Case {
                name: "rooted input without drive",
                input: r"\delivery\socket.rs",
                base: r"C:\workspace",
                expected: r"\delivery\socket.rs",
            },
        ];
        check_cases(&cases);
    }
}
