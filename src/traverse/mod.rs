//! Traverse paths and or mod declaration paths and manifest entry points.
//!
//! Essentially collects all `Chunk`s used for parsing with an associated
//! `Origin`.

use super::*;
use crate::Documentation;

use crate::errors::*;

use fs_err as fs;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub(crate) fn cwd() -> Result<PathBuf> {
    std::env::current_dir().wrap_err_with(|| eyre!("Missing cwd!"))
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

fn extract_modules_recurse_collect<P: AsRef<Path>>(
    path: P,
    acc: &mut HashSet<PathBuf>,
    mod_name: &str,
) -> Result<()> {
    let path = path.as_ref();
    let base = if let Some(base) = path.parent() {
        log::trace!("Parent path of {} is {}", path.display(), base.display());
        base.to_owned()
    } else {
        return Err(eyre!(
            "Must have a valid parent directory: {}",
            path.display()
        ));
    };
    let path1 = base.join(mod_name).join("mod.rs");
    let path2 = base.join(mod_name).with_extension("rs");
    let path3 = base
        .join(path.file_stem().expect("If parent exists, should work™"))
        .join(mod_name)
        .with_extension("rs");
    // avoid IO
    if acc.contains(&path1) || acc.contains(&path2) || acc.contains(&path3) {
        return Ok(());
    }
    match (path1.is_file(), path2.is_file(), path3.is_file()) {
        (true, false, false) => {
            let _ = acc.insert(path1);
        }
        (false, true, false) => {
            let _ = acc.insert(path2);
        }
        (false, false, true) => {
            let _ = acc.insert(path3);
        }
        (true, true, _) | (true, _, true) | (_, true, true) => {
            return Err(eyre!(
                "Detected both module entry files: {} and {} and {}",
                path1.display(),
                path2.display(),
                path3.display()
            ))
        }
        _ => log::trace!(
            "Neither file nor dir with mod.rs {} / {} / {}",
            path1.display(),
            path2.display(),
            path2.display()
        ),
    };
    Ok(())
}

fn extract_modules_recurse<P: AsRef<Path>>(
    path: P,
    stream: TokenStream,
) -> Result<HashSet<PathBuf>> {
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

    let mut acc = HashSet::with_capacity(16);

    #[derive(Debug, Clone)]
    enum SeekingFor {
        ModulKeyword,
        ModulName,
        ModulFin(String),
    }

    let mut state = SeekingFor::ModulKeyword;
    for tt in stream {
        match tt {
            TokenTree::Ident(ident) => match state {
                SeekingFor::ModulKeyword => {
                    if ident == "mod" {
                        state = SeekingFor::ModulName;
                    }
                }
                SeekingFor::ModulName => {
                    state = SeekingFor::ModulFin(ident.to_string());
                }
                _x => {
                    state = SeekingFor::ModulKeyword;
                }
            },
            TokenTree::Punct(punct) => {
                if let SeekingFor::ModulFin(ref mod_name) = state {
                    log::trace!("✨ Found a module: {mod_name}");
                    if punct.as_char() == ';' && punct.spacing() == Spacing::Alone {
                        extract_modules_recurse_collect(path, &mut acc, mod_name)?;
                    } else {
                        log::trace!("🍂 Either not alone or not a semi colon {punct:?} - incomplete mod {mod_name}");
                    }
                }
                state = SeekingFor::ModulKeyword;
            }
            TokenTree::Group(grp) => {
                state = SeekingFor::ModulKeyword;
                acc.extend(extract_modules_recurse(path, grp.stream())?.into_iter());
            }
            _y => {
                state = SeekingFor::ModulKeyword;
            }
        };
    }
    Ok(acc)
}

/// Read all `mod x;` declarations from a source file.
pub(crate) fn extract_modules_from_file<P: AsRef<Path>>(path: P) -> Result<HashSet<PathBuf>> {
    let path: &Path = path.as_ref();
    if let Some(path_str) = path.to_str() {
        let s = fs::read_to_string(path_str)?;
        let stream = syn::parse_str::<proc_macro2::TokenStream>(s.as_str())
            .wrap_err_with(|| eyre!("File {path_str} has syntax errors"))?;
        let acc = extract_modules_recurse(path, stream)?;
        log::debug!(
            "🥞 Recursed into {} modules from {}",
            acc.len(),
            path.display()
        );
        if log::log_enabled!(log::Level::Trace) {
            for path_rec in acc.iter() {
                log::trace!(
                    "🥞 recurse into {} from {}",
                    path_rec.display(),
                    path.display()
                );
            }
        }
        Ok(acc)
    } else {
        Err(eyre!("path must have a string representation"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CheckEntity {
    Markdown(PathBuf),
    Source(PathBuf, bool), // recurse is the bool
    ManifestDescription(PathBuf, String),
}

impl CheckEntity {
    #[allow(dead_code)]
    pub fn as_path(&self) -> &Path {
        match self {
            Self::Markdown(ref path) => path,
            Self::Source(ref path, _) => path,
            Self::ManifestDescription(ref path, _) => path,
        }
        .as_path()
    }
}

/// Returns both the parse manifest struct as well as the raw manifest string.
fn load_manifest<P: AsRef<Path>>(manifest_dir: P) -> Result<(cargo_toml::Manifest, String)> {
    let manifest_dir = manifest_dir.as_ref();
    let manifest_file = manifest_dir.join("Cargo.toml");
    // read to str first to provide better error messages
    let manifest_content = fs::read_to_string(&manifest_file)?;
    let mut manifest = cargo_toml::Manifest::from_str(manifest_content.as_str())
        .wrap_err_with(|| eyre!("Failed to parse manifest file {}", manifest_file.display()))?;

    // Load default products based on whatever exists on the filesystem.
    // This works for `src/main.rs` and `src/lib.rs` unless they are specified
    // in the manifest.
    if manifest.complete_from_path(&manifest_file).is_err()
        && manifest.complete_from_path(manifest_dir).is_err()
    {
        log::debug!(
            "Complete from filesystem did not yield new information for manifest {}",
            manifest_file.display()
        );
    }
    // Unfortunately we need this, the above does not
    // complete an existing `[lib]` without a path specified.
    // BUG: https://gitlab.com/crates.rs/cargo_toml/-/issues/5
    //
    // Required to assure the further invariant, that all
    // products have a valid `path`. Without a path it gets
    // removed from the product set.
    if let Some(ref mut lib) = manifest.lib {
        if lib.path.is_none() {
            lib.path = Some("src/lib.rs".to_owned())
        }
    }
    Ok((manifest, manifest_content))
}

/// can convert manifest with or without Cargo.toml into the directory that
/// contains the manifest
fn to_manifest_dir<P: AsRef<Path>>(manifest_dir: P) -> Result<PathBuf> {
    let manifest_dir: &Path = manifest_dir.as_ref();
    let manifest_dir = if manifest_dir.ends_with("Cargo.toml") {
        manifest_dir.parent().unwrap()
    } else {
        manifest_dir
    };
    fs::canonicalize(manifest_dir)
        .wrap_err_with(|| eyre!("Failed to canonicalize path {}", manifest_dir.display()))
}

/// Extract all cargo manifest products / build targets.
fn extract_products(
    manifest: &cargo_toml::Manifest,
    manifest_dir: &Path,
) -> Result<HashSet<CheckEntity>> {
    let iter = manifest.bin.clone().into_iter().chain(manifest.lib.clone());

    let items = iter
        .filter_map(|product| {
            if product.path.is_none() {
                log::warn!(
                    "Missing path for product {:?}, should have been filled earlier.",
                    product.name
                )
            }
            product.path
        })
        // cargo_toml's complete is not very truthfull
        .filter(|path_str| {
            let p = manifest_dir.join(PathBuf::from(path_str));
            let is_file = p.is_file();
            if !is_file {
                log::debug!("File listed by cargo-toml does not exist: {}", p.display());
            }
            is_file
        })
        .map(|path_str| CheckEntity::Source(manifest_dir.join(path_str), true))
        .collect::<HashSet<CheckEntity>>();

    log::trace!("📜 explicit manifest products {items:?}");
    Ok(items)
}

fn extract_readme(
    manifest: &cargo_toml::Manifest,
    manifest_dir: &Path,
) -> Result<Option<CheckEntity>> {
    Ok(manifest
        .package
        .as_ref()
        .and_then(|package| package.readme.get().ok().and_then(|x| x.as_path()))
        .and_then(|readme| {
            let readme = PathBuf::from(readme);
            if readme.is_file() {
                Some(CheckEntity::Markdown(manifest_dir.join(readme)))
            } else {
                log::warn!(
                    "📜 read-me file declared in Cargo.toml {} is not a file",
                    readme.display()
                );
                None
            }
        }))
}

fn extract_description(
    manifest: &cargo_toml::Manifest,
    manifest_dir: &Path,
    manifest_content: &str,
) -> Result<Option<CheckEntity>> {
    Ok(manifest
        .package
        .as_ref()
        .and_then(|package| package.description.as_ref())
        .map(|_description| {
            CheckEntity::ManifestDescription(
                manifest_dir.join("Cargo.toml"),
                manifest_content.to_owned(),
            )
        }))
}

fn handle_manifest<P: AsRef<Path>>(
    manifest_dir: P,
    skip_readme: bool,
) -> Result<HashSet<CheckEntity>> {
    let manifest_dir = to_manifest_dir(manifest_dir)?;
    log::trace!("📜 Handle manifest in dir: {}", manifest_dir.display());

    let manifest_dir = manifest_dir.as_path();
    let (manifest, manifest_content) = load_manifest(manifest_dir).wrap_err_with(|| {
        eyre!(
            "Failed to load manifest from dir {}",
            manifest_dir.display()
        )
    })?;

    let mut acc = extract_products(&manifest, manifest_dir).wrap_err_with(|| {
        eyre!(
            "Failed to extract products from manifest {}",
            manifest_dir.display()
        )
    })?;

    if !skip_readme {
        let v = extract_readme(&manifest, manifest_dir).wrap_err_with(|| {
            eyre!(
                "Failed to extract description from manifest {}",
                manifest_dir.display()
            )
        })?;
        acc.extend(v);
    }

    // TODO not quite ready for prime time
    if false {
        let v = extract_description(&manifest, manifest_dir, &manifest_content).wrap_err_with(
            || {
                eyre!(
                    "Failed to extract description from manifest {}",
                    manifest_dir.display()
                )
            },
        )?;
        acc.extend(v);
    }

    if let Some(workspace) = manifest.workspace {
        log::trace!("🪆 Handling manifest workspace");
        workspace
            .members
            .into_iter()
            .try_for_each::<_, Result<()>>(|member_entry_glob| {
                let member_dir_glob = manifest_dir.join(&member_entry_glob);

                let back_to_glob = member_dir_glob.as_os_str().to_str().ok_or_else(|| {
                    eyre!(
                        "Failed to convert path to str for member directory {}",
                        member_dir_glob.display()
                    )
                })?;
                let member_dirs = glob::glob(back_to_glob)?;
                log::debug!("🪆 Handing manifest member: {member_entry_glob}");
                for member_dir in member_dirs {
                    let member_dir = member_dir?;
                    log::trace!(
                        "🪆 Handling manifest member glob resolved: {}",
                        member_dir.display()
                    );
                    if let Ok((member_manifest, _member_manifest_content)) =
                        load_manifest(&member_dir).wrap_err_with(|| {
                            eyre!(
                                "Failed to load manifest from member directory {}",
                                member_dir.display()
                            )
                        })
                    {
                        if let Ok(member) = extract_products(&member_manifest, &member_dir) {
                            acc.extend(member.into_iter());
                        } else {
                            bail!(
                                "Workspace member {} product extraction failed",
                                member_dir.display()
                            );
                        }
                    } else {
                        log::warn!(
                            "🪆 Opening manifest from member failed {}",
                            member_dir.display()
                        );
                    }
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

    log::debug!("Running on inputs {paths:?} / recursive={recurse}");

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
        log::debug!("Processing {} -> {}", path_in.display(), path.display());
        path.canonicalize().ok()
    }));

    log::debug!("Running on absolute dirs {flow:?}");

    // stage 2 - check for manifest, .rs , .md files and directories
    let mut files_to_check = Vec::with_capacity(64);
    while let Some(path) = flow.pop_front() {
        let x = if let Ok(meta) = path.metadata() {
            if meta.is_file() {
                match path.file_name().and_then(|x| x.to_str()) {
                    Some(file_name) if file_name == "Cargo.toml" => Extraction::Manifest(path),
                    Some(file_name) if file_name.ends_with(".md") => Extraction::Markdown(path),
                    Some(file_name) if file_name.ends_with(".rs") => Extraction::Source(path),
                    _ => {
                        // This branch is commonly entered when ran on a non-cargo
                        // path.
                        // Potentially become mdbook aware
                        // <https://github.com/drahnr/cargo-spellcheck/issues/273>
                        log::debug!(
                            "Unknown file type encountered, skipping path: {}",
                            path.display()
                        );
                        continue;
                    }
                }
            } else if meta.is_dir() {
                let cargo_toml = to_manifest_dir(&path).unwrap().join("Cargo.toml");
                if cargo_toml.is_file() {
                    Extraction::Manifest(cargo_toml)
                } else if recurse {
                    // keep walking directories and feed the path back
                    // if recursing is wanted
                    // and if it doesn't contain a manifest file
                    match fs::read_dir(path) {
                        Err(err) => log::warn!("Listing directory contents {err} failed"),
                        Ok(entries) => {
                            for entry in entries.flatten() {
                                let path = entry.path();
                                // let's try with that path again
                                flow.push_back(path);
                            }
                        }
                    }
                    continue;
                } else {
                    match fs::read_dir(path) {
                        Err(err) => log::warn!("Listing directory contents {err} failed"),
                        Ok(entries) => {
                            for entry in entries.flatten() {
                                let path = entry.path();
                                // let's try attempt with that .rs file
                                // if we end up here, recursion is off already
                                if path.is_file() {
                                    flow.push_back(path);
                                }
                            }
                        }
                    }
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

    log::debug!("Found a total of {} files to check ", files_to_check.len());

    // stage 3 - resolve the manifest products and workspaces, warn about missing
    let files_to_check = files_to_check
        .into_iter()
        .try_fold::<Vec<_>, _, Result<_>>(Vec::with_capacity(64), |mut acc, tagged_path| {
            match tagged_path {
                Extraction::Manifest(ref cargo_toml_path) => {
                    let manifest_list = handle_manifest(cargo_toml_path, skip_readme)?;
                    acc.extend(manifest_list);
                }
                Extraction::Missing(ref missing_path) => log::warn!(
                    "File passed as argument or listed in Cargo.toml manifest does not exist: {}",
                    missing_path.display()
                ),
                Extraction::Source(path) => acc.push(CheckEntity::Source(path, recurse)),
                Extraction::Markdown(path) => acc.push(CheckEntity::Markdown(path)),
            }
            Ok(acc)
        })?;

    // stage 4 - expand from the passed source files, if recursive, recurse down the module train
    let docs = files_to_check.into_iter().try_fold(
        Documentation::new(),
        |mut docs, check_entity| -> Result<_> {
            match check_entity {
                CheckEntity::Source(path, recurse) => {
                    let content: String = fs::read_to_string(&path)?;
                    docs.add_rust(
                        ContentOrigin::RustSourceFile(path.clone()),
                        content.as_str(),
                        true,
                        dev_comments,
                    )?;

                    if recurse {
                        let iter =
                            Vec::from_iter(traverse(path.as_path(), true, dev_comments)?.flat_map(
                                |documentation| {
                                    // Filter out duplicate _chunks_
                                    // that `extend` would happily duplicate.
                                    documentation
                                        .into_iter()
                                        .filter(|(origin, _chunks)| !docs.contains_key(origin))
                                },
                            ));
                        docs.extend(iter);
                    }
                }
                CheckEntity::Markdown(path) => {
                    let content = fs::read_to_string(&path)
                        .wrap_err_with(|| eyre!("Common mark / markdown file does not exist"))?;
                    if content.is_empty() {
                        bail!("Common mark / markdown file is empty")
                    }
                    docs.add_commonmark(ContentOrigin::CommonMarkFile(path), content.as_str())?;
                }
                CheckEntity::ManifestDescription(path, content) => {
                    if content.is_empty() {
                        bail!("Cargo.toml manifest description field is empty")
                    }
                    docs.add_cargo_manifest_description(path, content.as_str())?;
                }
            }
            Result::Ok(docs)
        },
    )?;

    Result::Ok(docs)
}

#[cfg(test)]
mod tests {
    use super::*;

    use itertools::Itertools;

    const TEST_FILE_FRAGMENTS: &str = "src/nested/fragments.rs";
    const TEST_FILE_SIMPLE: &str = "src/nested/fragments/simple.rs";
    #[test]
    fn obtain_modules() {
        let _ = env_logger::builder()
            .is_test(true)
            .filter(None, log::LevelFilter::Trace)
            .try_init();

        assert_eq!(
            extract_modules_from_file(demo_dir().join(TEST_FILE_FRAGMENTS))
                .expect("fragments.rs must exist"),
            maplit::hashset![
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
        let _ = env_logger::builder()
            .is_test(true)
            .filter(None, log::LevelFilter::Trace)
            .try_init();

        let (manifest, dir) = demo_dir_manifest();
        assert_eq!(
            extract_products(&manifest, &dir).expect("Must succeed"),
            maplit::hashset![
                CheckEntity::Source(demo_dir().join("src/main.rs"), true),
                CheckEntity::Source(demo_dir().join("src/lib.rs"), true),
            ]
        );
        assert_eq!(
            extract_readme(&manifest, &dir).expect("Must succeed"),
            Some(CheckEntity::Markdown(demo_dir().join("README.md")),)
        );

        let manifest_content = include_str!("../../demo/Cargo.toml").to_owned();
        assert_matches::assert_matches!(
            extract_description(&manifest, &dir, manifest_content.as_str()),
            Ok(Some(CheckEntity::ManifestDescription(
                path,
                _
            ))) => {
                assert_eq!(path, demo_dir().join("Cargo.toml"));
            }
        );
    }

    fn demo_dir() -> PathBuf {
        manifest_dir().join("demo")
    }

    fn demo_dir_manifest() -> (cargo_toml::Manifest, PathBuf) {
        (
            load_manifest(demo_dir())
                .expect("Demo dir manifest must exist")
                .0,
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

    macro_rules! assert_hashset_eq_pretty {
        ($left:expr, $right:expr) => {
            let left: HashSet<_> = $left;
            let right: HashSet<_> = $right;
            let delta = left.difference(&right).collect::<Vec<_>>();
            let gamma = right.difference(&left).collect::<Vec<_>>();

            if !delta.is_empty() {
                eprintln!("Left does not contain {:?}", &delta[..]);
            }
            if !gamma.is_empty() {
                eprintln!("Right does not contain {:?}", &gamma[..]);
            }
            assert_eq!(
                Vec::from_iter(left.iter().sorted()),
                Vec::from_iter(right.iter().sorted())
            );
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

            assert_hashset_eq_pretty!(
                into_hashset(
                    docs.into_iter()
                        .map(|x| {
                            let path = x.0.as_path();
                            log::trace!("prefix: {}  --- item: {}", demo_dir().display(), path.display());
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
            // "Cargo.toml",
            "README.md",
            "src/lib.rs",
            "src/main.rs",
            "src/nested/again/mod.rs",
            "src/nested/again/code.rs",
            "src/nested/fragments/enumerate.rs",
            "src/nested/fragments/simple.rs",
            "src/nested/fragments.rs",
            "src/nested/justone.rs",
            "src/nested/justtwo.rs",
            "src/nested/mod.rs",
            "member/true/lib.rs",
            "member/procmacro/src/lib.rs",
        ]);
    }

    extract_test!(traverse_source_dir_1, ["src"] + false => [
        "src/lib.rs",
        "src/main.rs"]);

    extract_test!(traverse_source_dir_rec, ["src"] + true => [
        "src/lib.rs",
        "src/main.rs",
        "src/nested/again/mod.rs",
        "src/nested/again/code.rs",
        "src/nested/fragments/enumerate.rs",
        "src/nested/fragments/simple.rs",
        "src/nested/fragments.rs",
        "src/nested/justone.rs",
        "src/nested/justtwo.rs",
        "src/nested/mod.rs"
    ]);

    extract_test!(traverse_manifest_dir_rec, ["."] + true => [
        // "Cargo.toml",
        "README.md",
        "src/lib.rs",
        "src/main.rs",
        "src/nested/again/mod.rs",
        "src/nested/again/code.rs",
        "src/nested/fragments/enumerate.rs",
        "src/nested/fragments/simple.rs",
        "src/nested/fragments.rs",
        "src/nested/justone.rs",
        "src/nested/justtwo.rs",
        "src/nested/mod.rs",
        "member/true/lib.rs",
        "member/procmacro/src/lib.rs",
    ]);

    extract_test!(traverse_manifest_rec, ["Cargo.toml"] + true => [
        // "Cargo.toml",
        "README.md",
        "src/lib.rs",
        "src/main.rs",
        "src/nested/again/mod.rs",
        "src/nested/again/code.rs",
        "src/nested/fragments/enumerate.rs",
        "src/nested/fragments/simple.rs",
        "src/nested/fragments.rs",
        "src/nested/justone.rs",
        "src/nested/justtwo.rs",
        "src/nested/mod.rs",
        "member/true/lib.rs",
        "member/procmacro/src/lib.rs",
    ]);

    extract_test!(traverse_nested_mod_rs_1, ["src/nested/mod.rs"] + false => [
        "src/nested/mod.rs"
    ]);

    extract_test!(traverse_nested_mod_rs_rec, ["src/nested/mod.rs"] + true => [
        "src/nested/again/mod.rs",
        "src/nested/again/code.rs",
        "src/nested/fragments/enumerate.rs",
        "src/nested/fragments/simple.rs",
        "src/nested/fragments.rs",
        "src/nested/justone.rs",
        "src/nested/justtwo.rs",
        "src/nested/mod.rs"
    ]);

    extract_test!(traverse_dir_wo_manifest, ["member"] + true => [
        "member/true/lib.rs",
        "member/true/README.md",
        // "member/true/Cargo.toml",
        "member/procmacro/src/lib.rs",
        // "member/procmacro/Cargo.toml",
        "member/stray.rs",
    ]);
}
