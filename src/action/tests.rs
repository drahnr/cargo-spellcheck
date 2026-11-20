use super::*;
use std::convert::TryInto;

macro_rules! verify_correction {
    ($text:literal, $bandaids:expr, $expected:literal) => {
        let mut sink: Vec<u8> = Vec::with_capacity(1024);

        apply_patches(
            $bandaids.into_iter().map(|bandaid| Patch::from(bandaid)),
            $text,
            &mut sink,
        )
        .expect("Line correction must work in unit test!");

        assert_eq!(String::from_utf8_lossy(sink.as_slice()), $expected);
    };
}

#[test]
fn patch_full() {
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    let patches = vec![
        Patch::Replace {
            replace_span: Span {
                start: LineColumn { line: 1, column: 6 },
                end: LineColumn {
                    line: 2,
                    column: 12,
                },
            },
            replacement: "& Omega".to_owned(),
        },
        Patch::Insert {
            insert_at: LineColumn { line: 3, column: 0 },
            content: "Icecream truck".to_owned(),
        },
    ];
    verify_correction!(
        r#"Alpha beta gamma
zeta eta beta.
"#,
        patches,
        r#"Alpha & Omega.
Icecream truck"#
    );
}

#[test]
fn patch_replace_1() {
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();
    let bandaids = vec![Patch::Replace {
        replace_span: (1_usize, 0..1).try_into().unwrap(),
        replacement: "Y".to_owned(),
    }];
    verify_correction!("T🐠🐠U", bandaids, "Y🐠🐠U");
}

#[test]
fn patch_replace_2() {
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();
    let bandaids = vec![Patch::Replace {
        replace_span: (1_usize, 1..3).try_into().unwrap(),
        replacement: "Y".to_owned(),
    }];
    verify_correction!("T🐠🐠U", bandaids, "TYU");
}

#[test]
fn patch_replace_3() {
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();
    let bandaids = vec![Patch::Replace {
        replace_span: (1_usize, 3..4).try_into().unwrap(),
        replacement: "Y".to_owned(),
    }];
    verify_correction!("T🐠🐠U", bandaids, "T🐠🐠Y");
}

#[test]
fn patch_injection_1() {
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    let patches = vec![Patch::Insert {
        insert_at: LineColumn {
            line: 1_usize,
            column: 0,
        },
        content: "Q".to_owned(),
    }];
    verify_correction!("A🐢C", patches, "QA🐢C");
}

#[test]
fn patch_injection_2() {
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    let patches = vec![Patch::Insert {
        insert_at: LineColumn {
            line: 1_usize,
            column: 2,
        },
        content: "Q".to_owned(),
    }];
    verify_correction!("A🐢C", patches, "A🐢QC");
}
#[test]
fn patch_injection_3() {
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    let patches = vec![Patch::Insert {
        insert_at: LineColumn {
            line: 1_usize,
            column: 3,
        },
        content: "Q".to_owned(),
    }];
    verify_correction!("A🐢C", patches, "A🐢CQ");
}
