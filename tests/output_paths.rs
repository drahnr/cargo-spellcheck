use fs_err as fs;
#[cfg(feature = "spellbook")]
use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "cargo-spellcheck-output-paths-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self {
            root: root.canonicalize().unwrap(),
        };
        fixture.write(
            "Cargo.toml",
            "[package]\nname = \"output-paths-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\nreadme = false\n",
        );
        fixture.write(
            "spellcheck.toml",
            "[Spellbook]\nlang = \"en_US\"\nskip_os_lookups = true\nuse_builtin = true\n",
        );
        for path in ["src/lib.rs", "a/src/lib.rs", "b/src/lib.rs"] {
            fixture.write(path, "//! This is mispelled.\n");
        }
        fixture
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-spellcheck"))
            .current_dir(&self.root)
            .args(["--cfg", "spellcheck.toml"])
            .args(args)
            .env_remove("CARGO_SPELLCHECK")
            .env_remove("RUST_LOG")
            .env("NO_COLOR", "1")
            .env("CLICOLOR_FORCE", "0")
            .env("TOKIO_WORKER_THREADS", "1")
            .stdin(Stdio::null())
            .output()
            .expect("spawn cargo-spellcheck");
        assert!(
            output.status.success(),
            "cargo-spellcheck {args:?} exited with {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        output
    }

    fn assert_list(&self, args: &[&str], expected: &[&str]) {
        let output = self.run(args);
        let stdout = String::from_utf8(output.stdout).unwrap();
        // The CLI restores the cursor on exit, even when stdout is captured.
        let stdout = console::strip_ansi_codes(&stdout);
        let mut actual: Vec<_> = stdout
            .lines()
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .collect();
        let mut expected: Vec<_> = expected.iter().map(PathBuf::from).collect();
        // Traversal order is independent of the display-path policy.
        actual.sort();
        expected.sort();
        assert_eq!(actual, expected, "arguments: {args:?}");
    }

    #[cfg(feature = "spellbook")]
    fn assert_check(&self, args: &[&str], expected: &[&str]) {
        let output = self.run(args);
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        let stdout = console::strip_ansi_codes(&stdout);
        let stderr = console::strip_ansi_codes(&stderr);
        let mut locations: Vec<_> = stdout
            .lines()
            .filter_map(|line| line.trim().strip_prefix("--> "))
            .map(str::to_owned)
            .collect();
        let mut expected_locations: Vec<_> = expected
            .iter()
            .map(|path| format!("{}:1", Path::new(path).display()))
            .collect();
        locations.sort();
        expected_locations.sort();
        assert_eq!(locations, expected_locations, "stdout:\n{stdout}");
        assert!(stdout.contains("error: spellcheck(Spellbook)"), "{stdout}");
        assert!(stdout.contains("mispelled"), "{stdout}");

        let mut progress: Vec<_> = stderr
            .lines()
            .filter_map(|line| line.split_once("❌ ").map(|(_, path)| path.to_owned()))
            .collect();
        let mut expected_progress: Vec<_> = expected
            .iter()
            .map(|path| format!("{} : 1", Path::new(path).display()))
            .collect();
        progress.sort();
        expected_progress.sort();
        assert_eq!(progress, expected_progress, "stderr:\n{stderr}");
        if expected.iter().all(|path| Path::new(path).is_relative()) {
            let root = self.root.to_str().unwrap();
            assert!(
                !stdout.contains(root),
                "absolute path in diagnostics: {stdout}"
            );
            assert!(
                !stderr.contains(root),
                "absolute path in progress: {stderr}"
            );
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn list_files_without_inputs_is_relative_to_cwd() {
    let fixture = Fixture::new();
    fixture.assert_list(&["list-files"], &["src/lib.rs"]);
}

#[test]
fn list_files_single_directory_is_relative_to_that_directory() {
    let fixture = Fixture::new();
    fixture.assert_list(
        &["list-files", "--recursive", "--skip-readme", "a"],
        &["src/lib.rs"],
    );
    let absolute = fixture.root.join("a");
    fixture.assert_list(
        &["list-files", "--recursive", absolute.to_str().unwrap()],
        &["src/lib.rs"],
    );
}

#[test]
fn list_files_single_file_is_relative_to_its_parent() {
    let fixture = Fixture::new();
    fixture.assert_list(&["list-files", "a/src/lib.rs"], &["lib.rs"]);
    let absolute = fixture.root.join("a/src/lib.rs");
    fixture.assert_list(&["list-files", absolute.to_str().unwrap()], &["lib.rs"]);
}

#[test]
fn list_files_multiple_directories_preserve_original_prefixes() {
    let fixture = Fixture::new();
    fixture.assert_list(
        &["list-files", "--recursive", "a", "b"],
        &["a/src/lib.rs", "b/src/lib.rs"],
    );
}

#[test]
fn list_files_multiple_files_preserve_original_prefixes() {
    let fixture = Fixture::new();
    fixture.assert_list(
        &["list-files", "a/src/lib.rs", "b/src/lib.rs"],
        &["a/src/lib.rs", "b/src/lib.rs"],
    );
}

#[test]
fn list_files_mixed_inputs_preserve_original_prefixes() {
    let fixture = Fixture::new();
    fixture.assert_list(
        &["list-files", "--recursive", "a", "b/src/lib.rs"],
        &["a/src/lib.rs", "b/src/lib.rs"],
    );
}

#[test]
fn list_files_absolute_paths_override_all_inputs() {
    let fixture = Fixture::new();
    let root_file = fixture.root.join("src/lib.rs");
    let a_file = fixture.root.join("a/src/lib.rs");
    let b_file = fixture.root.join("b/src/lib.rs");
    let cases: &[(&[&str], &[&str])] = &[
        (&[], &[root_file.to_str().unwrap()]),
        (&["a"], &[a_file.to_str().unwrap()]),
        (&["a/src/lib.rs"], &[a_file.to_str().unwrap()]),
        (
            &["a", "b"],
            &[a_file.to_str().unwrap(), b_file.to_str().unwrap()],
        ),
        (
            &["a/src/lib.rs", "b/src/lib.rs"],
            &[a_file.to_str().unwrap(), b_file.to_str().unwrap()],
        ),
    ];
    for (inputs, expected) in cases {
        for prefix in [
            vec!["--absolute-paths", "list-files", "--recursive"],
            vec!["list-files", "--recursive", "--absolute-paths"],
        ] {
            let mut args = prefix;
            args.extend_from_slice(inputs);
            fixture.assert_list(&args, expected);
        }
    }
}

#[cfg(feature = "spellbook")]
#[test]
fn check_progress_and_diagnostics_follow_input_relative_paths() {
    let fixture = Fixture::new();
    let cases: &[(&[&str], &[&str])] = &[
        (&[], &["src/lib.rs"]),
        (&["a"], &["src/lib.rs"]),
        (&["a/src/lib.rs"], &["lib.rs"]),
        (&["a", "b"], &["a/src/lib.rs", "b/src/lib.rs"]),
        (
            &["a/src/lib.rs", "b/src/lib.rs"],
            &["a/src/lib.rs", "b/src/lib.rs"],
        ),
    ];
    for (inputs, expected) in cases {
        let mut args = vec![
            "-vv",
            "check",
            "--checkers",
            "spellbook",
            "--recursive",
            "--jobs",
            "1",
        ];
        args.extend_from_slice(inputs);
        fixture.assert_check(&args, expected);
    }
}

#[cfg(feature = "spellbook")]
#[test]
fn check_absolute_paths_override_progress_and_diagnostics() {
    let fixture = Fixture::new();
    let a_file = fixture.root.join("a/src/lib.rs");
    let b_file = fixture.root.join("b/src/lib.rs");
    for prefix in [
        vec!["-vv", "--absolute-paths", "check"],
        vec!["-vv", "check", "--absolute-paths"],
    ] {
        let mut args = prefix;
        args.extend_from_slice(&[
            "--checkers",
            "spellbook",
            "--recursive",
            "--jobs",
            "1",
            "a",
            "b",
        ]);
        fixture.assert_check(&args, &[a_file.to_str().unwrap(), b_file.to_str().unwrap()]);
    }
}
