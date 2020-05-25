//! Executes the actual path traversal and creating a token stream.
//!
//! Whatever.

use super::*;
use crate::Documentation;

use std::fs;

use log::{debug, info, trace, warn};

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

fn extract_cargo_toml_entry_files(cargo_toml: &Path) -> Vec<PathBuf> {
    unimplemented!("Extraction is not yet implemented, smth with `toml`");
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

fn extract_entry_points<P: AsRef<Path>>(manifest_dir: P) -> anyhow::Result<Vec<PathBuf>> {
    let manifest_dir = manifest_dir.as_ref();
    let mut manifest = cargo_toml::Manifest::from_path(manifest_dir.join("Cargo.toml"))?;
    // @todo verify which one is the sane one here, internally it calls `parent()`
    // but semantically it's not entirely clear.
    // manifest.complete_from_path(manifest_dir.join("Cargo.toml").as_path())?;
    manifest.complete_from_path(manifest_dir.join("src").as_path())?;
    Ok(dbg!(manifest)
        .bin
        .into_iter()
        .filter(|product| product.doctest)
        .filter_map(|product| dbg!(product.path))
        .map(|path_str| manifest_dir.join(path_str))
        .collect())
}

pub(crate) fn run(
    mode: Mode,
    paths: Vec<PathBuf>,
    recurse: bool,
    config: &Config,
) -> anyhow::Result<()> {
    // @todo extract bin and lib from toml to obtain the entry point files, from there resolve modules
    // @todo in case paths.len() == 1 && dir contains a `Cargo.toml` || file name and ends_with `Cargo.toml`
    // @todo honour recurse flag if path is a dir, otherwise error
    let cargo_tomls: Vec<_> = paths
        .iter()
        .filter_map(|path| {
            let meta = path.metadata().ok()?;
            if path.file_name() == Some("Cargo.toml".as_ref()) && meta.is_file() {
                Some(path.to_owned())
            } else if meta.is_dir() {
                let cargo_toml = path.with_file_name("Cargo.toml");
                if cargo_toml.is_file() {
                    Some(cargo_toml)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    let docs: Vec<Documentation> = if recurse {
        trace!("Recursive");
        paths
            .iter()
            .try_fold::<Vec<Documentation>, _, anyhow::Result<Vec<Documentation>>>(
                Vec::with_capacity(paths.len()),
                |mut acc, path| {
                    let content = fs::read_to_string(&path)?;
                    let stream = syn::parse_str(&content)?;
                    let path: String = path.to_str().unwrap().to_owned();
                    acc.push(Documentation::from((path, stream)));
                    Ok(acc)
                },
            )?
    } else {
        trace!("Single file");
        paths
            .iter()
            .try_fold::<Vec<Documentation>, _, anyhow::Result<Vec<Documentation>>>(
                Vec::with_capacity(paths.len()),
                |mut acc, path| {
                    let mut doc = traverse(path)?;
                    acc.append(&mut doc);
                    Ok(acc)
                },
            )?
    };

    let combined = Documentation::combine(docs);
    let suggestions = crate::checker::check(&combined, config)?;

    match mode {
        Mode::Fix => unimplemented!("Unsupervised fixing is not implemented just yet"),
        Mode::Check => {
            for suggestion in suggestions {
                eprintln!("{}", suggestion);
            }
        }
        Mode::Interactive => unimplemented!("Interactive pick & apply is not implemented just yet"),
    }

    Ok(())
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
            vec![PathBuf::from(TEST_FILE_SIMPLE).with_file_name("simple.rs")]
        );
    }

    #[test]
    fn manifest_entries() {
        assert_eq!(
            extract_entry_points(PathBuf::from(env!("CARGO_MANIFEST_DIR"))).expect("Must succeed"),
            vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/main.rs")]
        );
    }
}
