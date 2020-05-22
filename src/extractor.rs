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

pub(crate) fn run(mode: Mode, paths: Vec<PathBuf>, recurse: bool, config: &Config) -> anyhow::Result<()> {
    // @todo extract bin and lib from toml to obtain the entry point files, from there resolve modules
    // @todo in case paths.len() == 1 && dir contains a `Cargo.toml` || file name and ends_with `Cargo.toml`
    // @todo honour recurse flag if path is a dir, otherwise error
    let cargo_tomls: Vec<_> = paths.iter().filter_map(|path| {
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

    }).collect();


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
