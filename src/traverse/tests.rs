use super::*;

use itertools::Itertools;

/// Temporary module tree removed even when an extraction assertion fails.
struct WarningFixture(PathBuf);

impl Drop for WarningFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn recursive_extraction_retains_warnings_from_empty_and_partial_documentation() {
    let directory = std::env::temp_dir().join(format!(
        "cargo-spellcheck-extraction-warnings-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir(&directory).unwrap();
    let fixture = WarningFixture(directory);
    let root_source = r#"/// Transport entry.
mod opaque;
mod conditional;"#;
    let opaque_source = r#"#[doc = include_str!("transport-guide.md")]
struct TransportGuide;"#;
    let conditional_source = r#"/// Packet handling.
#[cfg_attr(feature = "trace", doc = include_str!("tracing.md"))]
struct Packet;"#;
    let root_path = fixture.0.join("lib.rs");
    let opaque_path = fixture.0.join("opaque.rs");
    let conditional_path = fixture.0.join("conditional.rs");
    fs::write(&root_path, root_source).unwrap();
    fs::write(&opaque_path, opaque_source).unwrap();
    fs::write(&conditional_path, conditional_source).unwrap();
    let paths = vec![root_path];
    let config = Config::default();
    let expected_warning_origins = HashSet::<_>::from_iter([
        ContentOrigin::RustSourceFile(opaque_path.clone()),
        ContentOrigin::RustSourceFile(conditional_path.clone()),
    ]);
    let expected_warning_count = expected_warning_origins.len();
    let expected_kind = doc_chunks::ExtractionWarningKind::OpaqueDocumentation;
    let expected_empty_chunk_count = 0;
    let expected_partial_chunk_count = 1;
    let documentation = extract(paths, true, true, false, &config).unwrap();
    assert_eq!(documentation.warnings().len(), expected_warning_count);
    let actual_warning_origins = HashSet::<_>::from_iter(
        documentation
            .warnings()
            .iter()
            .map(|located| located.origin.clone()),
    );
    assert_eq!(actual_warning_origins, expected_warning_origins);
    for located in documentation.warnings() {
        assert_eq!(located.warning.kind, expected_kind);
    }
    let opaque_origin = ContentOrigin::RustSourceFile(opaque_path);
    let conditional_origin = ContentOrigin::RustSourceFile(conditional_path);
    assert_eq!(
        documentation.get(&opaque_origin).unwrap().len(),
        expected_empty_chunk_count
    );
    assert_eq!(
        documentation.get(&conditional_origin).unwrap().len(),
        expected_partial_chunk_count
    );
}

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
