use super::*;

fn demo_dir() -> PathBuf {
    manifest_dir().join("demo")
}

#[test]
fn traverse_main_rs() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();
    let manifest_path = demo_dir().join("src/main.rs");

    let expect = indexmap::indexset! {
        "src/main.rs",
        "src/lib.rs",
        "src/nested/mod.rs",
        "src/nested/justone.rs",
        "src/nested/justtwo.rs",
        "src/nested/again/mod.rs",
        "src/nested/again/code.rs",
        "src/nested/fragments.rs",
        "src/nested/fragments/enumerate.rs",
        "src/nested/fragments/simple.rs",
    }
    .into_iter()
    .map(|sub| demo_dir().join(sub))
    .collect::<indexmap::set::IndexSet<PathBuf>>();

    let found = TraverseModulesIter::new(manifest_path.as_path())
        .expect("Must succeed to traverse file tree.")
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
