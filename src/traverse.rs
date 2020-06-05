//! Executes the actual path traversal and creating a token stream.
//!
//! Whatever.

use super::*;
use crate::Documentation;

use std::fs;

use log::{trace, warn};

use std::path::{Path, PathBuf};

/// TODO do this incremental, first parse the firstt file
/// and extract all mod declarations and filter the subdirs and files based on
/// the modules names. That way stale files can be avoided.
pub(crate) fn traverse(path: &Path) -> anyhow::Result<Vec<Documentation>> {
    let sources = walkdir::WalkDir::new(path)
        .max_depth(45)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry: &walkdir::DirEntry| -> bool { entry.file_type().is_file() })
        .filter_map(|entry| Some(entry.path().to_str()?.to_owned()))
        .filter(|path| path.ends_with(".rs"))
        .collect::<Vec<String>>();

    let documentation = sources
        .iter()
        .filter_map(|path: &String| -> Option<Documentation> {
            fs::read_to_string(path)
                .ok()
                .and_then(|content: String| syn::parse_str(&content).ok())
                .map(|stream| Documentation::from((path, stream)))
        })
        .filter(|documentation| !documentation.is_empty())
        .collect();
    Ok(documentation)
}

use proc_macro2::Spacing;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;

fn extract_modules_inner<P: AsRef<Path>>(
    path: P,
    stream: TokenStream,
) -> anyhow::Result<Vec<PathBuf>> {
    let path: &Path = path.as_ref();

    // Ident {
    //     sym: mod,
    // },
    // Ident {
    //     sym: M,
    // },
    // Punct {
    //     op: ';',
    //     spacing: Alone,
    // },

    let base = if let Some(base) = path.parent() {
        trace!("Parent path is {}", base.display());
        base.to_owned()
    } else {
        return Err(anyhow::anyhow!(
            "Must have a valid parent directory: {}",
            path.display()
        ));
    };

    #[derive(Debug, Clone)]
    enum SeekingFor {
        ModulKeyword,
        ModulName,
        ModulFin(String),
    }

    let mut acc = Vec::with_capacity(16);
    let mut state = SeekingFor::ModulKeyword;
    for tree in stream {
        match tree {
            TokenTree::Ident(ident) => match state {
                SeekingFor::ModulKeyword => {
                    if ident == "mod" {
                        state = SeekingFor::ModulName;
                    }
                }
                SeekingFor::ModulName => {
                    state = SeekingFor::ModulFin(ident.to_string());
                }
                _ => {}
            },
            TokenTree::Punct(punct) => {
                if let SeekingFor::ModulFin(mod_name) = state {
                    if punct.as_char() == ';' && punct.spacing() == Spacing::Alone {
                        let path1 = base.join(&mod_name).join("mod.rs");
                        let path2 = base.join(mod_name).with_extension("rs");
                        match (path1.is_file(), path2.is_file()) {
                            (true, false) => acc.push(path1),
                            (false, true) => acc.push(path2),
                            (true, true) => {
                                return Err(anyhow::anyhow!(
                                    "Detected both module entry files: {} and {}",
                                    path1.display(),
                                    path2.display()
                                ))
                            }
                            _ => trace!(
                                "Neither file not dir with mod.rs {} / {}",
                                path1.display(),
                                path2.display()
                            ),
                        };
                    } else {
                        trace!("Either not alone or not a semi colon {:?}", punct);
                    }
                }
                state = SeekingFor::ModulKeyword;
            }
            _ => {
                state = SeekingFor::ModulKeyword;
            }
        };
    }
    Ok(acc)
}

/// Read all `mod x;` declarations from a source file.
fn extract_modules_from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Vec<PathBuf>> {
    let path = path.as_ref();
    if let Some(path_str) = path.to_str() {
        let s = std::fs::read_to_string(path_str)
            .map_err(|e| anyhow::anyhow!("Failed to read file content: {}", e))?;
        let stream =
            syn::parse_str(s.as_str()).map_err(|_e| anyhow::anyhow!("has syntax errors"))?;
        extract_modules_inner(path.to_owned(), stream)
    } else {
        Err(anyhow::anyhow!("path must have a string representation"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CheckItem {
    Markdown(PathBuf),
    Source(PathBuf),
    ManifestDescription(String),
}

/// Extract all cargo manifest products / build targets.
// @todo code with an enum to allow source and markdown files
fn extract_products<P: AsRef<Path>>(manifest_dir: P) -> anyhow::Result<Vec<CheckItem>> {
    let manifest_dir = manifest_dir.as_ref();
    let manifest_file = manifest_dir.join("Cargo.toml");
    let mut manifest = cargo_toml::Manifest::from_path(&manifest_file).map_err(|e| {
        anyhow::anyhow!(
            "Failed to parse manifest file {}: {}",
            manifest_file.display(),
            e
        )
    })?;
    // @todo verify which one is the sane one here, internally it calls `parent()`
    // but semantically it's not entirely clear.
    // manifest.complete_from_path(manifest_dir.join("Cargo.toml").as_path())?;
    manifest.complete_from_path(&manifest_file).map_err(|e| {
        anyhow::anyhow!(
            "Failed to complete manifest info {}: {}",
            manifest_file.display(),
            e
        )
    })?;

    let mut items = manifest
        .bin
        .into_iter()
        .filter(|product| product.doctest)
        .filter_map(|product| product.path)
        .map(|path_str| CheckItem::Source(manifest_dir.join(path_str)))
        .collect::<Vec<CheckItem>>();

    if let Some(package) = manifest.package {
        if let Some(readme) = package.readme {
            let readme = PathBuf::from(readme);
            if readme.is_file() {
                items.push(CheckItem::Markdown(manifest_dir.join(readme)))
            } else {
                warn!(
                    "README.md defined in Cargo.toml {} is not a file",
                    readme.display()
                );
            }
        }
        if let Some(description) = package.description {
            items.push(CheckItem::ManifestDescription(description.to_owned()))
        }
    }
    Ok(items)
}

/// Execute execute execute.
pub(crate) fn collect(
    mut paths: Vec<PathBuf>,
    mut recurse: bool,
    _config: &Config,
) -> anyhow::Result<Documentation> {
    let cwd = std::env::current_dir().map_err(|_e| anyhow::anyhow!("Missing cwd!"))?;

    // if there are no arguments, pretend to be told to check the whole project
    if paths.is_empty() {
        // @todo also traverse parent dirs
        paths.push(cwd.join("Cargo.toml"));
        recurse = true;
    }

    #[derive(Debug, Clone)]
    enum Extraction {
        Manifest(PathBuf),
        Missing(PathBuf),
        Source(PathBuf),
        Markdown(PathBuf),
    }

    // convert all `Cargo.toml` manifest files to their respective product files
    // so after this conversion all of them are considered
    let items: Vec<_> = paths
        .into_iter()
        .map(|path| {
            let path = if  path.is_absolute() { path } else { cwd.join(path) };
            if let Ok(meta) = path.metadata() {
                if meta.is_file() {
                    match path.file_name().map(|x| x.to_str()).flatten() {
                        Some(file_name) if file_name == "Cargo.toml" => Extraction::Manifest(path),
                        Some(file_name) if file_name.ends_with(".md") => Extraction::Markdown(path),
                        _ => Extraction::Source(path),
                    }
                } else if meta.is_dir() {
                    let cargo_toml = path.with_file_name("Cargo.toml");
                    if cargo_toml.is_file() {
                        Extraction::Manifest(cargo_toml)
                    } else {
                        // @todo should we just collect all .rs files here instead?
                        Extraction::Missing(cargo_toml)
                    }
                } else {
                    Extraction::Missing(path)
                }
            } else {
                Extraction::Missing(path)
            }
        })
        .try_fold::<Vec<_>, _, anyhow::Result<_>>(
            Vec::with_capacity(64),
            |mut acc, tagged_path| {
                match tagged_path {
                    Extraction::Manifest(ref cargo_toml_path) => {
                        let manifest_list = extract_products(cargo_toml_path.parent().unwrap())?;
                        acc.extend(manifest_list);
                    },
                    Extraction::Missing(ref missing_path) => warn!("File passed as argument or listed in Cargo.toml manifest does not exist: {}", missing_path.display()),
                    Extraction::Source(path) => acc.push(CheckItem::Source(path)),
                    Extraction::Markdown(path) => acc.push(CheckItem::Markdown(path)),
                }
                Ok(acc)
            },
        )?;

    let docs: Vec<Documentation> = if recurse {
        let mut path_collection = indexmap::IndexSet::<_>::with_capacity(64);

        // @todo merge this with the `Documentation::from` to reduce parsing of the file twice
        let mut dq = std::collections::VecDeque::<CheckItem>::with_capacity(64);
        dq.extend(items.into_iter());
        while let Some(item) = dq.pop_front() {
            if let CheckItem::Source(path) = item {
                let modules = extract_modules_from_file(&path)?;
                if path_collection.insert(CheckItem::Source(path.to_owned())) {
                    dq.extend(modules.into_iter().map(CheckItem::Source));
                } else {
                    warn!("Already visited module");
                }
            }
        }

        trace!("Recursive");
        let n = path_collection.len();
        path_collection
            .into_iter()
            .try_fold::<Vec<Documentation>, _, anyhow::Result<Vec<Documentation>>>(
                Vec::with_capacity(n),
                |mut acc, item| {
                    match item {
                        CheckItem::Source(path) => {
                            let content = fs::read_to_string(&path)?;
                            let stream = syn::parse_str(&content)?;
                            acc.push(Documentation::from((path, stream)));
                        }
                        _ => unimplemented!("Did not impl this just yet"),
                    }
                    Ok(acc)
                },
            )?
    } else {
        trace!("Single file");
        items
            .iter()
            .try_fold::<Vec<Documentation>, _, anyhow::Result<Vec<Documentation>>>(
                Vec::with_capacity(items.len()),
                |mut acc, item| {
                    match item {
                        CheckItem::Source(path) => {
                            let mut doc = traverse(path)?;
                            acc.append(&mut doc);
                        }
                        _ => {
                            // @todo generate Documentation structs from non-file sources
                        }
                    }
                    Ok(acc)
                },
            )?
    };

    let combined = Documentation::combine(docs);

    Ok(combined)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_FILE_FRAGMENTS: &str = "src/tests/fragments.rs";
    const TEST_FILE_SIMPLE: &str = "src/tests/simple.rs";
    #[test]
    fn obtain_modules() {
        let _ = env_logger::try_init();

        assert_eq!(
            extract_modules_from_file(PathBuf::from(TEST_FILE_FRAGMENTS)).unwrap(),
            vec![
                PathBuf::from(TEST_FILE_SIMPLE).with_file_name("simple.rs"),
                PathBuf::from(TEST_FILE_SIMPLE).with_file_name("enumerate.rs"),
            ]
        );
    }

    #[test]
    fn manifest_entries() {
        assert_eq!(
            extract_products(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).expect("Must succeed"),
            vec![
                CheckItem::Source(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/main.rs")),
                CheckItem::Markdown(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("README.md")),
                CheckItem::ManifestDescription(
                    "Checks all doc comments for spelling mistakes".to_string()
                ),
            ]
        );
    }
}
