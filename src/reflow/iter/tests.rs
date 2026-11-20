use super::*;

/// `indentations` represent the indentation of individual lines in
/// `content`.
fn verify_reflow(
    content: &'static str,
    expected: &'static str,
    max_line_width: usize,
    unbreakables: Vec<Range>,
    indentations: Vec<usize>,
) {
    let indentations = indentations
        .into_iter()
        .map(Indentation::<'static>::new)
        .collect::<Vec<_>>();
    let expected = expected
        .lines()
        .enumerate()
        .map(|(idx, line)| (idx + 1, line));

    let mut gluon = Gluon::new(content, max_line_width, &indentations);

    gluon.add_unbreakables(unbreakables);

    for ((line_no, line_content, _), (expected_no, expected_content)) in
        gluon.clone().zip(expected.clone())
    {
        assert_eq!(line_no, expected_no);
        assert_eq!(dbg!(line_content), dbg!(expected_content));
    }

    assert_eq!(dbg!(gluon).count(), expected.count());
}

mod tokeneer {
    use super::*;

    fn verify(content: &'static str, expected: &[&'static str], unbreakables: Vec<Range>) {
        let mut expected_iter = expected.iter();
        let tokeneer = Tokeneer::new(content, unbreakables);
        for (idx, (_char_range, _byte_range, s)) in tokeneer.enumerate() {
            let expected = expected_iter
                .next()
                .expect("Must be of equal length at index");
            println!("idx {idx} : {s} <=> {expected}");
            assert_eq!(s.as_ref(), *expected);
        }
    }

    #[test]
    fn smilies() {
        const CONTENT: &str = "🍇🌡 🌤";
        const EXPECTED: &[&str] = &["🍇🌡", "🌤"];
        verify(CONTENT, EXPECTED, std::iter::once(0..2).collect());
    }

    #[test]
    fn multi_char() {
        const CONTENT: &str = "abc xyz qwert";
        const EXPECTED: &[&str] = &["abc", "xyz", "qwert"];
        verify(CONTENT, EXPECTED, vec![]);
    }

    #[test]
    fn partial_covered_word_unbreakable() {
        const CONTENT: &str = "abc xyz qwert";
        const EXPECTED: &[&str] = &["abc xyz", "qwert"];
        verify(CONTENT, EXPECTED, std::iter::once(2..5).collect());
    }
}

mod gluon {
    use super::*;
    #[test]
    fn wrap_too_long_fluid() {
        const CONTENT: &str = "something kinda too long for a single line";
        const EXPECTED: &str = r#"something kinda too long for a
single line"#;
        verify_reflow(CONTENT, EXPECTED, 30usize, vec![], vec![0]);
    }

    #[test]
    fn wrap_too_short_fluid() {
        const CONTENT: &str = r#"something
kinda
too
short
for
a
single
line"#;

        const EXPECTED: &str = r#"something kinda too short for
a single line"#;

        verify_reflow(CONTENT, EXPECTED, 30usize, vec![], vec![0; 8]);
    }

    #[test]
    fn wrap_just_fine() {
        const CONTENT: &str = r#"just fine, no action required 🐱"#;
        const EXPECTED: &str = CONTENT;

        verify_reflow(CONTENT, EXPECTED, 40usize, vec![], vec![0]);
    }

    #[test]
    fn wrap_too_long_unbreakable() {
        const CONTENT: &str = "something kinda too Xong for a singlX line";
        const EXPECTED: &str = r#"something kinda too
Xong for a singlX line"#;
        verify_reflow(
            CONTENT,
            EXPECTED,
            30usize,
            std::iter::once(20..37).collect(),
            vec![0],
        );
    }

    #[test]
    fn spaces_and_tabs() {
        const CONTENT: &str = "        something     kinda       ";
        const EXPECTED: &str = r#"something kinda"#;
        verify_reflow(CONTENT, EXPECTED, 20usize, vec![], vec![0]);
    }

    #[test]
    fn deep_indentation_too_long() {
        const CONTENT: &str = r#"deep indentation"#;
        const EXPECTED: &str = r#"deep
indentation"#;
        // 20 < 31 = 4 + 1 + 11 + 15
        verify_reflow(CONTENT, EXPECTED, 20usize, vec![], vec![15]);
    }

    #[test]
    fn deep_indentation_too_short() {
        let _ = env_logger::builder()
            .is_test(true)
            .filter(None, log::LevelFilter::Trace)
            .try_init();

        const CONTENT: &str = r#"deep
indentation"#;
        const EXPECTED: &str = r#"deep indentation"#;
        // 22 > 21 = 4 + 1 + 11 + 5
        verify_reflow(CONTENT, EXPECTED, 22usize, vec![], vec![5, 5]);
    }
}
