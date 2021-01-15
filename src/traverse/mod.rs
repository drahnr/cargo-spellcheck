//! Travers paths and or mod declaration paths and manifest entry points.
//!
//! Essentially collects all `Chunk`s used for parsing with an associated `Origin`.

use super::*;
use crate::Documentation;

use anyhow::{anyhow, bail, Error, Result};
use log::{debug, trace, warn};

use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn cwd() -> Result<PathBuf> {
    std::env::current_dir().map_err(|_e| anyhow::anyhow!("Missing cwd!"))
}

#[cfg(test)]
fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

use std::collections::VecDeque;

mod iter;
pub use iter::*;

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
                            .join(
                                path.file_stem()
                                    .expect("If parent exists, should work (TM)"),
                            )
                            .join(mod_name)
                            .with_extension("rs");
                        match (path1.is_file(), path2.is_file(), path3.is_file()) {
                            (true, false, false) => acc.push(path1),
                            (false, true, false) => acc.push(path2),
                            (false, false, true) => acc.push(path3),
                            (true, true, _) | (true, _, true) | (_, true, true) => {
                                return Err(anyhow::anyhow!(
                                    "Detected both module entry files: {} and {} and {}",
                                    path1.display(),
                                    path2.display(),
                                    path3.display()
                                ))
                            }
                            _ => trace!(
                                "Neither file nor dir with mod.rs {} / {} / {}",
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
pub(crate) fn extract_modules_from_file<P: AsRef<Path>>(path: P) -> Result<Vec<PathBuf>> {
    let path: &Path = path.as_ref();
    if let Some(path_str) = path.to_str() {
        let s = std::fs::read_to_string(path_str).map_err(|e| {
            Error::from(e).context(anyhow!("Failed to read file content of {}", path_str))
        })?;
        let stream = syn::parse_str::<proc_macro2::TokenStream>(s.as_str())
            .map_err(|e| Error::from(e).context(anyhow!("File {} has syntax errors", path_str)))?;
        extract_modules_inner(path.to_owned(), stream)
    } else {
        Err(anyhow::anyhow!("path must have a string representation"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CheckEntity {
    Markdown(PathBuf),
    Source(PathBuf, bool), // recurse is the bool
    ManifestDescription(String),
}

fn load_manifest<P: AsRef<Path>>(manifest_dir: P) -> Result<cargo_toml::Manifest> {
    let manifest_dir = manifest_dir.as_ref();
    let manifest_file = manifest_dir.join("Cargo.toml");
    // read to str first to provide better error messages
    let manifest_content = std::fs::read_to_string(&manifest_file).map_err(|e| {
        anyhow::anyhow!("Failed to open manifest file {}", manifest_file.display()).context(e)
    })?;
    let mut manifest = cargo_toml::Manifest::from_str(manifest_content.as_str()).map_err(|e| {
        anyhow::anyhow!("Failed to parse manifest file {}", manifest_file.display()).context(e)
    })?;
    // load default products based on whatever exists on the filesystem
    if manifest.complete_from_path(&manifest_file).is_err() {
        if manifest.complete_from_path(manifest_dir).is_err() {
            debug!(
                "Complete from filesystem did not yield new information for manifest {}",
                manifest_file.display()
            );
        }
    }
    Ok(manifest)
}

/// can convert manifest with or without Cargo.toml into the dir that contains the manifest
fn to_manifest_dir<P: AsRef<Path>>(manifest_dir: P) -> Result<PathBuf> {
    let manifest_dir: &Path = manifest_dir.as_ref();
    if manifest_dir.ends_with("Cargo.toml") {
        manifest_dir.parent().unwrap()
    } else {
        manifest_dir
    }
    .canonicalize()
    .map_err(|e| {
        Error::from(e).context(anyhow!(
            "Failed to canonicalize path {}",
            manifest_dir.display()
        ))
    })
}

/// Extract all cargo manifest products / build targets.
fn extract_products(
    manifest: &cargo_toml::Manifest,
    manifest_dir: &Path,
) -> Result<Vec<CheckEntity>> {
    let iter = manifest
        .bin
        .iter()
        .cloned()
        .chain(manifest.lib.iter().cloned().map(|x| x));

    let items = iter
        .filter(|product| product.doctest)
        .filter_map(|product| product.path)
        // cargo_toml's complete is not very truthfull
        .filter(|path_str| manifest_dir.join(path_str).is_file())
        .map(|path_str| CheckEntity::Source(manifest_dir.join(path_str), true))
        .collect::<Vec<CheckEntity>>();

    trace!("manifest products {:?}", &items);
    Ok(items)
}

fn extract_readme(
    manifest: &cargo_toml::Manifest,
    manifest_dir: &Path,
) -> Result<Vec<CheckEntity>> {
    let mut acc = Vec::with_capacity(2);
    if let Some(package) = manifest.package.clone() {
        if let Some(readme) = package.readme {
            let readme = PathBuf::from(readme);
            if readme.is_file() {
                acc.push(CheckEntity::Markdown(manifest_dir.join(readme)));
            } else {
                warn!(
                    "README.md defined in Cargo.toml {} is not a file",
                    readme.display()
                );
            }
        }
        if let Some(description) = package.description {
            acc.push(CheckEntity::ManifestDescription(description.to_owned()));
        }
    }
    Ok(acc)
}

fn handle_manifest<P: AsRef<Path>>(manifest_dir: P, skip_readme: bool) -> Result<Vec<CheckEntity>> {
    let manifest_dir = to_manifest_dir(manifest_dir)?;
    trace!("Handle manifest in dir: {}", manifest_dir.display());

    let manifest_dir = manifest_dir.as_path();
    let manifest = load_manifest(manifest_dir).map_err(|e| {
        anyhow!(
            "Failed to load manifest from dir {}",
            manifest_dir.display()
        )
        .context(e)
    })?;

    let mut acc = extract_products(&manifest, &manifest_dir).map_err(|e| {
        anyhow!(
            "Failed to extract products from manifest {}",
            manifest_dir.display()
        )
        .context(e)
    })?;

    if !skip_readme {
        let v = extract_readme(&manifest, &manifest_dir).map_err(|e| {
            anyhow!(
                "Failed to extract readme / description from manifest {}",
                manifest_dir.display()
            )
            .context(e)
        })?;
        acc.extend(v);
    }

    if let Some(workspace) = manifest.workspace {
        trace!("Handling manifest workspace");
        workspace
            .members
            .into_iter()
            .try_for_each::<_, Result<()>>(|item| {
                let member_dir = manifest_dir.join(&item);
                trace!(
                    "Handling manifest member {} -> {}",
                    &item,
                    member_dir.display()
                );
                if let Ok(member_manifest) = load_manifest(&member_dir).map_err(|e| {
                    anyhow!(
                        "Failed to load manifest from member directory {}",
                        member_dir.display()
                    )
                    .context(e)
                }) {
                    if let Ok(member) = extract_products(&member_manifest, &member_dir) {
                        acc.extend(member.into_iter());
                    } else {
                        warn!("Workspace member {} product extraction failed", item);
                    }
                } else {
                    warn!("Opening manifest from member failed {}", item);
                }
                Ok(())
            })?;
    }
    Ok(acc)
}

/// Extract all chunks from
pub(crate) fn extract(
    mut paths: Vec<PathBuf>,
    mut recurse: bool,
    skip_readme: bool,
    dev_comments: bool,
    _config: &Config,
) -> Result<Documentation> {
    let cwd = cwd()?;
    // if there are no arguments, pretend to be told to check the whole project
    if paths.is_empty() {
        paths.push(cwd.clone());
        recurse = true;
    }

    debug!("Running on inputs {:?} / recursive={}", &paths, recurse);

    #[derive(Debug, Clone)]
    enum Extraction {
        Manifest(PathBuf),
        Missing(PathBuf),
        Source(PathBuf),
        Markdown(PathBuf),
    }

    // stage 1 - obtain canonical paths
    let mut flow = VecDeque::<PathBuf>::with_capacity(32);
    flow.extend(paths.into_iter().filter_map(|path_in| {
        let path = if path_in.is_absolute() {
            path_in.to_owned()
        } else {
            cwd.join(&path_in)
        };
        debug!("Processing {} -> {}", path_in.display(), path.display());
        path.canonicalize().ok()
    }));

    debug!("Running on absolute dirs {:?} ", &flow);

    // stage 2 - check for manifest, .rs , .md files and directories
    let mut files_to_check = Vec::with_capacity(64);
    while let Some(path) = flow.pop_front() {
        let x = if let Ok(meta) = path.metadata() {
            if meta.is_file() {
                match path.file_name().map(|x| x.to_str()).flatten() {
                    Some(file_name) if file_name == "Cargo.toml" => Extraction::Manifest(path),
                    Some(file_name) if file_name.ends_with(".md") => Extraction::Markdown(path),
                    Some(file_name) if file_name.ends_with(".rs") => Extraction::Source(path),
                    _ => {
                        warn!("Unexpected item made it into the items {}", path.display());
                        continue;
                    }
                }
            } else if meta.is_dir() {
                let cargo_toml = to_manifest_dir(&path).unwrap().join("Cargo.toml");
                if cargo_toml.is_file() {
                    Extraction::Manifest(cargo_toml)
                } else {
                    // TODO should we just collect all .rs files here instead?

                    // we know it's a directory, and we limit the entries to 0 levels,
                    // will cause to yield all "^.*\.rs$" files in that dir
                    // which is what we want in this case
                    flow.extend(TraverseModulesIter::with_depth_limit(&path, 0)?);
                    continue;
                }
            } else {
                Extraction::Missing(path)
            }
        } else {
            Extraction::Missing(path)
        };
        files_to_check.push(x);
    }

    debug!("Found a total of {} files to check ", files_to_check.len());

    // stage 3 - resolve the manifest products and workspaces, warn about missing
    let files_to_check = files_to_check
        .into_iter()
        .try_fold::<Vec<_>, _, Result<_>>(Vec::with_capacity(64), |mut acc, tagged_path| {
            match tagged_path {
                Extraction::Manifest(ref cargo_toml_path) => {
                    let manifest_list = handle_manifest(cargo_toml_path, skip_readme)?;
                    acc.extend(manifest_list);
                }
                Extraction::Missing(ref missing_path) => warn!(
                    "File passed as argument or listed in Cargo.toml manifest does not exist: {}",
                    missing_path.display()
                ),
                Extraction::Source(path) => acc.push(CheckEntity::Source(path, recurse)),
                Extraction::Markdown(path) => acc.push(CheckEntity::Markdown(path)),
            }
            Ok(acc)
        })?;

    // stage 4 - expand from the passed source files, if recursive, recurse down the module train
    let combined: Documentation = files_to_check
        .into_iter()
        .try_fold::<Documentation, _, Result<Documentation>>(
            Documentation::new(),
            |mut docs, item| {
                match item {
                    CheckEntity::Source(path, recurse) => {
                        if recurse {
                            let iter = traverse(path.as_path(), dev_comments)?;
                            docs.extend(iter);
                        } else {
                            let content: String = fs::read_to_string(&path).map_err(|e| {
                                anyhow!("Failed to read {}", path.display()).context(e)
                            })?;
                            docs.add_rust(
                                ContentOrigin::RustSourceFile(path.to_owned()),
                                content.as_str(),
                                dev_comments,
                            )
                            .unwrap_or_else(|_e| {
                                log::error!("BUG: Failed to create cluster for {}", path.display())
                            });
                        }
                    }
                    CheckEntity::Markdown(path) => {
                        let content = std::fs::read_to_string(&path).map_err(|e| {
                            anyhow!("Common mark / markdown file does not exist").context(e)
                        })?;
                        if content.len() < 1 {
                            bail!("Common mark / markdown file is empty")
                        }
                        docs.add_commonmark(
                            ContentOrigin::CommonMarkFile(path.to_owned()),
                            content.as_str(),
                        )?;
                    }
                    other => {
                        warn!("Did not impl handling of {:?} type files", other);
                        // TODO generate Documentation structs from non-file sources
                    }
                }
                Ok(docs)
            },
        )?;

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
        let (manifest, dir) = demo_dir_manifest();
        assert_eq!(
            extract_products(&manifest, &dir).expect("Must succeed"),
            vec![
                CheckEntity::Source(demo_dir().join("src/main.rs"), true),
                CheckEntity::Source(demo_dir().join("src/lib.rs"), true),
            ]
        );
        assert_eq!(
            extract_readme(&manifest, &dir).expect("Must succeed"),
            vec![
                CheckEntity::Markdown(demo_dir().join("README.md")),
                CheckEntity::ManifestDescription(
                    "A silly demo with plenty of spelling mistakes for cargo-spellcheck demos and CI".to_string()
                ),
            ]
        );
    }

    fn demo_dir() -> PathBuf {
        manifest_dir().join("demo")
    }

    fn demo_dir_manifest() -> (cargo_toml::Manifest, PathBuf) {
        (
            load_manifest(demo_dir()).expect("Demo dir manifest must exist"),
            demo_dir(),
        )
    }

    use std::collections::HashSet;
    use std::hash::Hash;

    fn into_hashset<I, J, T>(source: I) -> HashSet<T>
    where
        I: IntoIterator<Item = T, IntoIter = J>,
        J: Iterator<Item = T>,
        T: Hash + Eq,
    {
        source.into_iter().collect::<HashSet<T>>()
    }

    macro_rules! pathset {
        ( $($x:expr),* $(,)? ) => {
            {
                let mut temp_set = HashSet::new();
                $(
                    temp_set.insert(PathBuf::from($x));
                )*
                temp_set
            }
        };
    }

    macro_rules! extract_test {

        ($name:ident, [ $( $path:literal ),* $(,)?] + $recurse: expr => [ $( $file:literal ),* $(,)?] ) => {

            #[test]
            fn $name() {
                extract_test!([ $( $path ),* ] + $recurse => [ $( $file ),* ]);
            }
        };

        ([ $( $path:literal ),* $(,)?] + $recurse: expr => [ $( $file:literal ),* $(,)?] ) => {
                    let _ = env_logger::builder()
            .is_test(true)
            .filter(None, log::LevelFilter::Trace)
            .try_init();

            let docs = extract(
                vec![
                    $(
                        demo_dir().join($path)
                    )*
                ],
                $recurse,
                false,
                true,
                &Config::default(),
            )
            .expect("Must be able to extract demo dir");
            assert_eq!(
                into_hashset(
                    docs.into_iter()
                        .map(|x| {
                            let path = x.0.as_path();
                            trace!("prefix: {}  --- item: {}", demo_dir().display(), path.display());
                            path.strip_prefix(demo_dir()).expect("Must have common prefix").to_owned()
                        })
                    ),
                pathset![
                    $(
                        ($file).to_owned(),
                    )*
                ]
            );

        };
    }

    #[test]
    fn traverse_manifest_1() {
        extract_test!(["Cargo.toml"] + false => [
            "README.md",
            "src/lib.rs",
            "src/main.rs",
            "src/nested/again/mod.rs",
            "src/nested/fragments/enumerate.rs",
            "src/nested/fragments/simple.rs",
            "src/nested/fragments.rs",
            "src/nested/justone.rs",
            "src/nested/justtwo.rs",
            "src/nested/mod.rs",
        ]);
    }

    extract_test!(traverse_source_dir_1, ["src"] + false => [
        "src/lib.rs",
        "src/main.rs"]);

    extract_test!(traverse_source_dir_rec, ["src"] + true => [
        "src/lib.rs",
        "src/main.rs",
        "src/nested/again/mod.rs",
        "src/nested/fragments/enumerate.rs",
        "src/nested/fragments/simple.rs",
        "src/nested/fragments.rs",
        "src/nested/justone.rs",
        "src/nested/justtwo.rs",
        "src/nested/mod.rs"
    ]);

    extract_test!(traverse_manifest_dir_rec, ["."] + true => [
        "README.md",
        "src/lib.rs",
        "src/main.rs",
        "src/nested/again/mod.rs",
        "src/nested/fragments/enumerate.rs",
        "src/nested/fragments/simple.rs",
        "src/nested/fragments.rs",
        "src/nested/justone.rs",
        "src/nested/justtwo.rs",
        "src/nested/mod.rs",
    ]);

    extract_test!(traverse_manifest_rec, ["Cargo.toml"] + true => [
        "README.md",
        "src/lib.rs",
        "src/main.rs",
        "src/nested/again/mod.rs",
        "src/nested/fragments/enumerate.rs",
        "src/nested/fragments/simple.rs",
        "src/nested/fragments.rs",
        "src/nested/justone.rs",
        "src/nested/justtwo.rs",
        "src/nested/mod.rs",
    ]);

    extract_test!(traverse_nested_mod_rs_1, ["src/nested/mod.rs"] + false => [
        "src/nested/mod.rs"
    ]);

    extract_test!(traverse_nested_mod_rs_rec, ["src/nested/mod.rs"] + true => [
        "src/nested/again/mod.rs",
        "src/nested/fragments/enumerate.rs",
        "src/nested/fragments/simple.rs",
        "src/nested/fragments.rs",
        "src/nested/justone.rs",
        "src/nested/justtwo.rs",
        "src/nested/mod.rs"
    ]);
}
