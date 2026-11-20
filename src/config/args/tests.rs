use super::*;
use assert_matches::assert_matches;

fn commandline_to_iter(s: &'static str) -> impl Iterator<Item = String> {
    s.split(' ').map(|s| s.to_owned())
}

lazy_static::lazy_static!(
    static ref SAMPLES: std::collections::HashMap<&'static str, Action> = maplit::hashmap!{
        // check (implicit)
        "cargo spellcheck" => Action::Check,
        "cargo spellcheck -vvvv" => Action::Check,
        "cargo-spellcheck" => Action::Check,
        "cargo-spellcheck -vvvv" => Action::Check,
        // check (explicit)
        "cargo spellcheck check -m 11" => Action::Check,
        "cargo-spellcheck check -m 9" => Action::Check,
        // reflow
        "cargo spellcheck reflow" => Action::Reflow,
        "cargo-spellcheck reflow" => Action::Reflow,
        // fix (deprecated)
        "cargo spellcheck --fix" => Action::Fix,
        "cargo-spellcheck --fix" => Action::Fix,
        // fix
        "cargo spellcheck fix" => Action::Fix,
        "cargo-spellcheck fix" => Action::Fix,
        "cargo-spellcheck fix -r file.rs" => Action::Fix,
        "cargo-spellcheck -q fix Cargo.toml" => Action::Fix,
        "cargo spellcheck -v fix Cargo.toml" => Action::Fix,

        // FIXME check it fully, against the unified args
        // TODO must implement an abstraction for the config file source for that
        // "cargo spellcheck completions --shell zsh" => Sub::Completions { shell: Shell::Zsh },
        // "cargo spellcheck completions --shell bash" => Sub::Completions { shell: Shell::Bash },
        // "cargo-spellcheck completions --shell zsh" => Sub::Completions { shell: Shell::Zsh },
        // "cargo-spellcheck completions --shell bash" => Sub::Completions { shell: Shell::Bash },
        // "cargo-spellcheck completions" => Sub::Completions { .. },
    };
);

#[test]
fn args() {
    for command in SAMPLES.keys() {
        assert_matches!(Args::parse(commandline_to_iter(command)), Ok(_));
    }
}

#[test]
fn deserialize_multiple_checkers() {
    let args = Args::parse(commandline_to_iter(
        "cargo spellcheck check --checkers=nlprules,hunspell",
    ))
    .expect("Parsing works. qed");
    assert_eq!(
        args.checkers(),
        Some(vec![CheckerType::NlpRules, CheckerType::Hunspell])
    );
}

#[test]
fn alt_fix_works() {
    let args_sub = Args::parse(commandline_to_iter("cargo spellcheck fix")).unwrap();
    let args_alt = Args::parse(commandline_to_iter("cargo spellcheck --fix")).unwrap();
    assert_eq!(args_sub.common(), args_alt.common());
    assert_eq!(args_sub.action(), args_alt.action());
}

#[test]
fn unify_ops_check() {
    let args = Args::parse(
        [
            "cargo",
            "spellcheck",
            "-vvvvv",
            "check",
            "--code=77",
            "--dev-comments",
            "--skip-readme",
        ]
        .iter()
        .map(ToOwned::to_owned)
        .map(ToOwned::to_owned),
    )
    .unwrap();
    let (unified, _config) = args.unified().unwrap();
    assert_matches!(unified,
        UnifiedArgs::Operate {
            action,
            config_path: _,
            dev_comments,
            skip_readme,
            recursive,
            paths,
            exit_code_override,
        } => {
            assert_eq!(Action::Check, action);
            assert_eq!(exit_code_override, 77);
            assert_eq!(dev_comments, true);
            assert_eq!(skip_readme, true);
            assert_eq!(recursive, false);
            assert_eq!(paths, Vec::<PathBuf>::new());
        }
    );
}

// FIXME checkers interpretation seems to have changed XXX
#[test]
fn unify_config() {
    let args = Args::parse(
        [
            "cargo-spellcheck",
            "--cfg=.config/spellcheck.toml",
            "config",
            "--checkers=NlpRules",
            "--overwrite",
        ]
        .iter()
        .map(ToOwned::to_owned)
        .map(ToOwned::to_owned),
    )
    .unwrap();
    let (unified, _config) = dbg!(args).unified().unwrap();
    assert_matches!(dbg!(unified),
        UnifiedArgs::Config {
            dest_config: ConfigWriteDestination::File { overwrite, path },
            checker_filter_set,
        } => {
            assert_eq!(path, PathBuf::from(".config/spellcheck.toml"));
            assert_eq!(checker_filter_set, Some(MultipleCheckerTypes(vec![CheckerType::NlpRules])));
            assert_eq!(overwrite, true);
        }
    );
}

#[test]
fn shell_check_env() {
    assert_matches!(load_shell_name("/usr/bin/zsh"), Ok(Shell::Zsh));
    assert_matches!(load_shell_name("zsh"), Ok(Shell::Zsh));
    assert_matches!(load_shell_name("fish"), Ok(Shell::Fish));

    static C1: &str = "cargo spellcheck completions --shell zsh";
    assert_matches!(Args::parse(commandline_to_iter(C1)), Ok(Args {
        command: Some(Sub::Completions { shell }),
        ..
    }) => {
        assert_eq!(shell.to_string(), "zsh")
    });

    // `SHELL` is builtin, and hence cannot be unset
    // so a negative test is impossible, an override works though
    // `std::env::remove_var("SHELL");`
    static C2: &str = "cargo spellcheck completions";

    std::env::set_var("SHELL", "/bin/fish");
    assert_matches!(Args::parse(commandline_to_iter(C2)), Ok(Args {
        command: Some(Sub::Completions { shell }),
        ..
    }) => {
        assert_eq!(shell.to_string(), "fish")
    });
}
