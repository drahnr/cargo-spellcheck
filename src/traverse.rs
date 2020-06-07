//! Executes the actual path traversal and creating a token stream.
//!
//! Whatever.

use super::*;
use crate::Documentation;

use std::fs;

use log::{trace, warn};

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Error, Result};

fn cwd() -> Result<PathBuf> {
    std::env::current_dir().map_err(|_e| anyhow::anyhow!("Missing cwd!"))
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

use std::collections::VecDeque;

/// An iterator traversing module hirarchies yielding paths

#[derive(Default, Debug, Clone)]
struct TraverseModulesIter {
    queue: VecDeque<PathBuf>,
}

impl TraverseModulesIter {
    pub fn with_multi<P, J, I>(entries: I) -> Result<Self>
    where
        P: AsRef<Path>,
        J: Iterator<Item = P>,
        I: IntoIterator<Item = P, IntoIter = J>,
    {
        let mut me = Self::default();
        for path in entries.into_iter().map(|p| { p.as_ref().to_owned() } ) {
            me.queue.push_back(path);
        }
        Ok(me)
    }

    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut me = Self::default();
        me.queue.push_back(path.as_ref().to_owned());
        Ok(me)
    }

    pub fn collect_modules(&mut self, path: &Path) -> Result<()> {
        if path.is_file() {
            self.queue.extend(extract_modules_from_file(path)?.into_iter());
        } else if path.is_dir() {
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
                }).try_for_each::<_,Result<()>>(|path| {
                    self.queue.extend(extract_modules_from_file(path)?.into_iter());
                    Ok(())
                })?;
        } else {
            warn!("Only dealing with dirs or files, dropping {}", path.display());
        }
        Ok(())
    }
}

impl Iterator for TraverseModulesIter {
    type Item = PathBuf;
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(path) = self.queue.pop_front() {
            self.collect_modules(dbg!(path.as_path()));
            Some(path)
        } else {
            None
        }
    }
}

fn traverse_source_files(path: &Path) -> Result<impl Iterator<Item = PathBuf>>  {
    Ok(TraverseModulesIter::new(path)?)
}

pub(crate) fn traverse(path: &Path) -> Result<impl Iterator<Item = Documentation>> {
    let it = TraverseModulesIter::new(path)?
        .filter_map(|path: PathBuf| -> Option<Documentation> {
            fs::read_to_string(&path)
                .ok()
                .and_then(|content: String| syn::parse_str(&content).ok())
                .map(|stream| Documentation::from((path, stream)))
        })
        .filter(|documentation| !documentation.is_empty());
    Ok(it)
}

use proc_macro2::Spacing;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;

fn extract_modules_inner<P: AsRef<Path>>(path: P, stream: TokenStream) -> Result<Vec<PathBuf>> {
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
        trace!("Parent path of {} is {}", path.display(), base.display());
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
                        let path2 = base.join(&mod_name).with_extension("rs");
                        let path3 = base
                            .join(path.file_stem().expect("If parent exists, should work (TM)"))
                            .join(mod_name)
                            .with_extension("rs");
                        match (path1.is_file(), path2.is_file(), path3.is_file()) {
                            (true, _, _) => acc.push(path1),
                            (false, true, _) => acc.push(path2),
                            (false, false, true) => acc.push(path3),
                            (true, true, _) | (true, _, true)  | (_, true, true) => {
                                return Err(anyhow::anyhow!(
                                    "Detected both module entry files: {} and {} and {}",
                                    path1.display(),
                                    path2.display(),
                                    path3.display()
                                ))
                            }
                            _ => trace!(
                                "Neither file not dir with mod.rs {} / {} / {}",
                                path1.display(),
                                path2.display(),
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
fn extract_modules_from_file<P: AsRef<Path>>(path: P) -> Result<Vec<PathBuf>> {
    let path: &Path = path.as_ref();
    if let Some(path_str) = path.to_str() {
        let s = std::fs::read_to_string(path_str).map_err(|e| {
            Error::from(e).context(anyhow!("Failed to read file content of {}", path_str))
        })?;
        let stream = syn::parse_str(s.as_str())
            .map_err(|e| Error::from(e).context(anyhow!("File {} has syntax errors", path_str)))?;
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
fn extract_products<P: AsRef<Path>>(manifest_dir: P) -> Result<Vec<CheckItem>> {
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
) -> Result<Documentation> {
    let cwd = cwd()?;
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
            let path = if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            };
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
        .try_fold::<Vec<_>, _, Result<_>>(Vec::with_capacity(64), |mut acc, tagged_path| {
            match tagged_path {
                Extraction::Manifest(ref cargo_toml_path) => {
                    let manifest_list = extract_products(cargo_toml_path.parent().unwrap())?;
                    acc.extend(manifest_list);
                }
                Extraction::Missing(ref missing_path) => warn!(
                    "File passed as argument or listed in Cargo.toml manifest does not exist: {}",
                    missing_path.display()
                ),
                Extraction::Source(path) => acc.push(CheckItem::Source(path)),
                Extraction::Markdown(path) => acc.push(CheckItem::Markdown(path)),
            }
            Ok(acc)
        })?;

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
            .try_fold::<Vec<Documentation>, _, Result<Vec<Documentation>>>(
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
            .try_fold::<Vec<Documentation>, _, Result<Vec<Documentation>>>(
                Vec::with_capacity(items.len()),
                |mut acc, item| {
                    match item {
                        CheckItem::Source(path) => {
                            acc.extend(traverse(path)?);
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

    const TEST_FILE_FRAGMENTS: &str = "src/nested/fragments.rs";
    const TEST_FILE_SIMPLE: &str = "src/nested/fragments/simple.rs";
    #[test]
    fn obtain_modules() {
        let _ = env_logger::try_init();

        assert_eq!(
            extract_modules_from_file(demo_dir().join(TEST_FILE_FRAGMENTS))
                .expect("fragments.rs must exist"),
            vec![
                demo_dir()
                    .join(TEST_FILE_SIMPLE)
                    .with_file_name("simple.rs"),
                demo_dir()
                    .join(TEST_FILE_SIMPLE)
                    .with_file_name("enumerate.rs"),
            ]
        );
    }

    #[test]
    fn manifest_entries() {
        assert_eq!(
            extract_products(demo_dir()).expect("Must succeed"),
            vec![
                CheckItem::Source(demo_dir().join("src/main.rs")),
                CheckItem::Markdown(demo_dir().join("README.md")),
                CheckItem::ManifestDescription(
                    "A silly demo with plenty of spelling mistakes for cargo-spellcheck demos and CI".to_string()
                ),
            ]
        );
    }

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

        let found = traverse_source_files(manifest_path.as_path())
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
