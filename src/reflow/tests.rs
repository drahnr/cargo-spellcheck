use super::*;
use crate::util::load_span_from;
use crate::{chyrp_up, fluff_up};
use crate::{Documentation, LineColumn, Span};

macro_rules! verify_reflow_inner {
    ($n:literal break [ $( $line:literal ),+ $(,)?] => $expected:literal) => {

        let _ = env_logger::Builder::new()
            .filter_level(log::LevelFilter::Debug)
            .filter(Some("reflow"), log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        const CONTENT: &str = fluff_up!($( $line ),+);
        let docs = Documentation::load_from_str(ContentOrigin::TestEntityRust, CONTENT, true, false);
        assert_eq!(docs.entry_count(), 1);
        let chunks = docs.get(&ContentOrigin::TestEntityRust).expect("Must contain dummy path");
        assert_eq!(dbg!(chunks).len(), 1);
        let chunk = &chunks[0];

        let range = 0..chunk.as_str().len();
        let indentation: Vec<_> = [3; 6].iter().map(|&n| {
            Indentation::<'static>::new(n)
        }).collect::<Vec<_>>();
        let unbreakables = Vec::new();
        let replacement = reflow_inner(
            chunk.as_str(),
            range,
            &unbreakables,
            &indentation,
            $n,
            &chunk.variant(),
            chunk.preferred_line_delimiter(),
        );

        if let Ok(Some(repl)) = replacement {
            // TODO: check indentation
            assert_eq!(repl, $expected);
        } else {
            for line in CONTENT.lines() {
                assert!(line.len() < $n);
            }
        }
    };
}

#[test]
fn reflow_replacement_from_chunk() {
    verify_reflow_inner!(80 break ["This module contains documentation that \
is too long for one line and moreover, it \
spans over mulitple lines such that we can \
test our rewrapping algorithm. With emojis: 🚤w🌴x🌋y🍈z🍉0",
    "Smart, isn't it? Lorem ipsum and some more \
    blanket text without any meaning"] =>
    r#"This module contains documentation that is too long for one line and
/// moreover, it spans over mulitple lines such that we can test our rewrapping
/// algorithm. With emojis: 🚤w🌴x🌋y🍈z🍉0 Smart, isn't it? Lorem ipsum and some more
/// blanket text without any meaning"#);
}

#[test]
fn reflow_inner_not_required() {
    verify_reflow_inner!(80 break ["This module contains documentation."] =>
        r#"This module contains documentation."#);
    {
        verify_reflow_inner!(39 break ["This module contains documentation",
            "which is split in two lines"] =>
            r#"This module contains documentation
/// which is split in two lines"#);
    }
}

/// With all patches applied.
///
/// Nothing to do, there is no line in need of a reflow.
///
/// Verify the individual patches are as expected.
macro_rules! reflow_content {
    ($max_line_width:literal break $content_type:expr, $content:expr => applied $expected:literal) => {
        const CFG: ReflowConfig = ReflowConfig {
            max_line_length: $max_line_width,
        };

        let _ = env_logger::Builder::new()
            .filter_level(log::LevelFilter::Debug)
            .filter(Some("reflow"), log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        let docs = Documentation::load_from_str($content_type, $content, true, false);
        assert_eq!(docs.entry_count(), 1);
        let chunks = docs.get(&$content_type).expect("Contains test data. qed");
        assert_eq!(dbg!(chunks).len(), 1);
        let chunk = &chunks[0];
        let _plain = chunk.erase_cmark(&Default::default());
        let suggestions = reflow(&$content_type, chunk, &CFG).expect("Reflow is working. qed");

        let patches = suggestions
            .into_iter()
            .filter_map(|suggestion| {
                suggestion.replacements.first()
                    .map(|replacement| {
                        let bandaid = crate::BandAid::from((
                            replacement.to_owned(),
                            &suggestion.span,
                        ));
                        bandaid
                    })
            })
            .map(|x| crate::Patch::from(x));

        let mut dest = Vec::with_capacity($content.len() * 3 / 2);
        crate::action::apply_patches(
            patches,
            $content,
            &mut dest,
        ).expect("Patches always apply nicely. qed");
        let s = std::string::String::from_utf8_lossy(&dest);
        eprintln!(">>>{}<<<", s);
        eprintln!(">>>{}<<<", $expected);
        assert_eq!(s, $expected, "Applied patches mismatch expected result");
    };
    ($max_line_width:literal break $content_type:expr, $content:expr => ok) => {
        const CFG: ReflowConfig = ReflowConfig {
            max_line_length: $max_line_width,
        };

        let _ = env_logger::Builder::new()
            .filter_level(log::LevelFilter::Debug)
            .filter(Some("reflow"), log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        let docs = Documentation::load_from_str($content_type, $content, true, false);
        assert_eq!(docs.entry_count(), 1);
        let chunks = docs.get(&$content_type).expect("Contains test data. qed");
        assert_eq!(dbg!(chunks).len(), 1);
        let chunk = &chunks[0];
        let _plain = chunk.erase_cmark(&Default::default());
        let suggestions = reflow(&$content_type, chunk, &CFG).expect("Reflow is working. qed");

        assert_eq!(
            dbg!(suggestions).len(),
            0,
            "The content is known to be ok, not in need of a reflow."
        );
    };
    ($max_line_width:literal break $content_type:expr, $content:expr => patches [ $( $expected:literal ),+ $(,)?]) => {
        const CFG: ReflowConfig = ReflowConfig {
            max_line_length: $max_line_width,
        };

        let _ = env_logger::Builder::new()
            .filter_level(log::LevelFilter::Debug)
            .filter(Some("reflow"), log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        let docs = Documentation::load_from_str($content_type, $content, true, false);
        assert_eq!(docs.entry_count(), 1);
        let chunks = docs.get(&$content_type).expect("Contains test data. qed");
        assert_eq!(dbg!(chunks).len(), 1);
        let chunk = &chunks[0];
        let _plain = chunk.erase_cmark(&Default::default());
        println!("reflow content:\n {:?}", $content);
        let suggestions = reflow(&$content_type, chunk, &CFG).expect("Reflow is working. qed");
        let patches = suggestions
            .into_iter()
            .filter_map(|suggestion| {
                suggestion.replacements.first()
                    .map(|replacement| {
                        let bandaid = crate::BandAid::from((
                            replacement.to_owned(),
                            &suggestion.span,
                        ));
                        bandaid
                    })
            })
            .map(|x| crate::Patch::from(x))
            .collect::<Vec<crate::Patch>>();
        let expected: Vec<&'static str> = vec![$( $expected ),+];

        // defer this check to later
        let patches_n = patches.len();
        let expected_n = expected.len();
        // it yields more info if we try to match as many as we can first
        for (idx, (patch, expected)) in patches.into_iter().zip(expected.into_iter()).enumerate() {
            log::info!("Patch #{idx} {patch:?}");
            assert_matches::assert_matches!(patch, crate::Patch::Replace {
                replacement,
                replace_span,
            } => {
                let content: &'static str = $content;
                let to_be_replaced = load_span_from(&mut content.as_bytes(), replace_span).expect("Test cases are well defined and do not cause out of bounds access. qed");
                log::info!("Patch #{idx} replaces {to_be_replaced:?} => {replacement:?}");
                assert_eq!(replacement.as_str(), expected, "Patch #{idx}");
            })
        }
        assert_eq!(patches_n, expected_n, "Number of suggestions mismatches expected patches");
    };
}

/// Run reflow on a set of lines that are `fluff_up`ed and match the resulting
/// `Patch`s replacement with an `expected` (a single literal, TODO allow
/// multiple).
macro_rules! reflow_fluff {
    ($n:literal break [ $( $line:literal ),+ $(,)?] => $( $expected:literal ),+ $(,)?) => {
        const CONTENT:&'static str = fluff_up!($( $line ),+);

        reflow_content!($n break ContentOrigin::TestEntityRust, CONTENT => patches [ $( $expected ),+ ]);
    };

    ($n:literal break [ $( $line:literal ),+ $(,)?] => ok) => {
        const CONTENT:&'static str = fluff_up!($( $line ),+);

        reflow_content!($n break ContentOrigin::TestEntityRust, CONTENT => ok);
    };
}

macro_rules! reflow_chyrp {
    ($n:literal break [ $( $line:literal ),+ $(,)?] => $( $expected:literal ),+ $(,)?) => {

        const CONTENT:&'static str = chyrp_up!($( $line ),+);

        reflow_content!($n break ContentOrigin::TestEntityRust, CONTENT => patches [ $( $expected ),+ ]);
    };
    ($n:literal break [ $( $line:literal ),+ $(,)?] => ok) => {

        const CONTENT:&'static str = chyrp_up!($( $line ),+);

        reflow_content!($n break ContentOrigin::TestEntityRust, CONTENT => ok);
    };
}

#[test]
fn reflow_into_suggestion() {
    reflow_fluff!(45 break ["This module contains documentation thats \
is too long for one line and moreover, \
it spans over mulitple lines such that \
we can test our rewrapping algorithm. \
Smart, isn't it? Lorem ipsum and some more \
blanket text without any meaning.",
    "But lets also see what happens if \
there are two consecutive newlines \
in one connected documentation span."] =>

r#"This module contains documentation thats
/// is too long for one line and moreover, it
/// spans over mulitple lines such that we
/// can test our rewrapping algorithm. Smart,
/// isn't it? Lorem ipsum and some more
/// blanket text without any meaning. But
/// lets also see what happens if there are
/// two consecutive newlines in one connected
/// documentation span."#);
}

#[test]
fn reflow_shorter_than_limit() {
    reflow_fluff!(80 break ["This module contains documentation that is ok for one line"] => ok);
}

#[test]
fn reflow_unchanged_first_line() {
    // Regression for #363: only the later lines change when reflowing this paragraph.
    const CONTENT: &str = r#"/// The first line of this paragraph is already filled up to the line width so
/// the second line is far too long and should be wrapped by the reflow command, but it is not.
pub fn a() {}
"#;

    reflow_content!(80 break ContentOrigin::TestEntityRust, CONTENT => applied
        r#"/// The first line of this paragraph is already filled up to the line width so
/// the second line is far too long and should be wrapped by the reflow command,
/// but it is not.
pub fn a() {}
"#);
}

fn apply_rust_reflow(source: &str, max_line_length: usize) -> (String, usize) {
    let origin = ContentOrigin::TestEntityRust;
    let docs = Documentation::load_from_str(origin.clone(), source, true, false);
    let chunks = docs
        .get(&origin)
        .expect("Rust documentation must be extracted");
    assert_eq!(chunks.len(), 1);
    assert!(chunks[0].is_reflow_safe());
    let checker = Reflow::new(ReflowConfig { max_line_length }).unwrap();
    let suggestions = checker.check(&origin, chunks).unwrap();
    (
        apply_reflow_patches(source, &suggestions),
        suggestions.len(),
    )
}

fn apply_reflow_patches(source: &str, suggestions: &[Suggestion<'_>]) -> String {
    let patches = suggestions.iter().map(|suggestion| {
        crate::Patch::from(crate::BandAid::from((
            suggestion.replacements[0].clone(),
            &suggestion.span,
        )))
    });
    let mut output = Vec::new();
    crate::action::apply_patches(patches, source, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

fn rust_doc_words(source: &str) -> Vec<String> {
    let file = syn::parse_file(source).expect("documentation must remain valid Rust");
    let syn::Item::Struct(item) = &file.items[0] else {
        panic!("expected a documented struct");
    };
    item.attrs
        .iter()
        .filter(|attr| attr.path().is_ident("doc"))
        .flat_map(|attr| {
            let syn::Meta::NameValue(meta) = &attr.meta else {
                panic!("expected a doc attribute value");
            };
            let syn::Expr::Lit(expr) = &meta.value else {
                panic!("expected a doc literal");
            };
            let syn::Lit::Str(literal) = &expr.lit else {
                panic!("expected a doc string");
            };
            literal
                .value()
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Reflow must reach the expected formatting in one pass and make no further suggestions.
#[test]
fn reflow_reaches_expected_output_and_is_idempotent() {
    /// A formatting contract exercised with both Unix and Windows source line endings.
    struct Case {
        name: &'static str,
        width: usize,
        source: &'static str,
        expected: &'static str,
    }
    let cases = [
        Case {
            name: "wrapped line comments",
            width: 30,
            source: r#"/// Packet routing preserves delivery order.
struct Delivery;
"#,
            expected: r#"/// Packet routing preserves
/// delivery order.
struct Delivery;
"#,
        },
        Case {
            name: "quoted doc attributes",
            width: 35,
            source: r#"#[doc = "Secure routes retain encrypted delivery metadata."]
struct SecureRoute;
"#,
            expected: r#"#[doc = "Secure routes retain"]
#[doc = "encrypted delivery"]
#[doc = "metadata."]
struct SecureRoute;
"#,
        },
        Case {
            name: "raw doc attributes",
            width: 35,
            source: r##"#[doc=r#"Cached routes retain recovery metadata."#]
struct CachedRoutes;
"##,
            expected: r##"#[doc=r#"Cached routes retain"#]
#[doc=r#"recovery metadata."#]
struct CachedRoutes;
"##,
        },
        Case {
            name: "indentation-only correction",
            width: 35,
            source: r#"/// Route delivery overview.
///    Transport stays ready.
struct Route;
"#,
            expected: r#"/// Route delivery overview.
/// Transport stays ready.
struct Route;
"#,
        },
    ];
    let line_delimiters = ["\n", "\r\n"];
    let expected_first_pass_suggestions = 1;
    let expected_second_pass_suggestions = 0;
    for Case {
        name,
        width,
        source,
        expected,
    } in cases
    {
        for delimiter in line_delimiters {
            let source = source.replace('\n', delimiter);
            let expected = expected.replace('\n', delimiter);
            // An exact first-pass expectation prevents a no-op reflow from passing this test.
            let (wrapped, first_count) = apply_rust_reflow(&source, width);
            assert_eq!(first_count, expected_first_pass_suggestions, "{name}");
            assert_eq!(wrapped, expected, "{name}, delimiter {delimiter:?}");
            // Reparse and recheck the formatted Rust through the real extraction/check/patch path.
            let (checked_again, second_count) = apply_rust_reflow(&wrapped, width);
            assert_eq!(second_count, expected_second_pass_suggestions, "{name}");
            assert_eq!(checked_again, expected, "{name}, delimiter {delimiter:?}");
        }
    }
}

/// Check later-line wrapping against the expected first line, width, and original prose.
fn assert_later_rust_lines_reflow(
    source: &str,
    max_line_length: usize,
    expected_first_line: &str,
) -> String {
    let before = rust_doc_words(source);
    let (wrapped, count) = apply_rust_reflow(source, max_line_length);
    assert_eq!(count, 1, "later lines must produce a suggestion: {source}");
    assert_ne!(wrapped, source);
    assert_eq!(wrapped.lines().next(), Some(expected_first_line));
    assert!(
        wrapped
            .lines()
            .all(|line| line.chars().count() <= max_line_length),
        "wrapped source exceeds width {max_line_length}:\n{wrapped}"
    );
    assert_eq!(rust_doc_words(&wrapped), before, "prose must be preserved");

    wrapped
}

#[test]
fn reflow_unchanged_first_doc_attribute() {
    // "Documentation" cannot fit after the introduction, so only later lines change.
    for (open, close) in [
        ("\"", "\""),
        ("r\"", "\""),
        ("r#\"", "\"#"),
        ("r###\"", "\"###"),
    ] {
        let source = format!(
            "#[doc={open}A short introduction.{close}]\n\
             #[doc={open}Documentation continues with enough ordinary words to require wrapping across several lines.{close}]\n\
             struct Example;\n"
        );
        assert_later_rust_lines_reflow(&source, 40, source.lines().next().unwrap());
    }
}

#[test]
fn reflow_unchanged_first_multiline_doc_string() {
    for (open, close) in [
        ("\"", "\""),
        ("r\"", "\""),
        ("r#\"", "\"#"),
        ("r###\"", "\"###"),
    ] {
        let source = format!(
            "#[doc={open}A short introduction.\n\
             Documentation continues with enough ordinary words to require wrapping across several lines.{close}]\n\
             struct Example;\n"
        );
        let first_line = format!("#[doc={open}A short introduction.{close}]");
        assert_later_rust_lines_reflow(&source, 40, &first_line);
    }
}

#[test]
fn reflow_unchanged_first_line_with_later_indentation_only() {
    let source = "/// A short introduction.\n///    Documentation stays.\nstruct Example;\n";
    assert_eq!(
        assert_later_rust_lines_reflow(source, 30, source.lines().next().unwrap()),
        "/// A short introduction.\n/// Documentation stays.\nstruct Example;\n"
    );

    for (open, close) in [("\"", "\""), ("r#\"", "\"#")] {
        let source = format!(
            "#[doc={open}A short introduction.{close}]\n\
             #[doc={open}   Documentation stays.{close}]\n\
             struct Example;\n"
        );
        let expected = source.replace("   Documentation", "Documentation");
        assert_eq!(
            assert_later_rust_lines_reflow(&source, 40, source.lines().next().unwrap()),
            expected
        );
    }
}

#[test]
fn reflow_multiple_lines() {
    reflow_fluff!(43 break ["This module contains documentation that is broken",
                        "into multiple short lines resulting in multiple spans."] =>
            r#"This module contains documentation that
/// is broken into multiple short lines
/// resulting in multiple spans."#);
}
#[test]
fn reflow_indentations() {
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    const CONTENT: &str = r#"
    /// 🔴 🍁
    /// 🤔
    struct Fluffy {};"#;

    const EXPECTED: &str = r#"🔴
    /// 🍁
    /// 🤔"#;

    const CONFIG: ReflowConfig = ReflowConfig {
        max_line_length: 10,
    };

    let docs = Documentation::load_from_str(ContentOrigin::TestEntityRust, CONTENT, true, false);
    assert_eq!(docs.entry_count(), 1);
    let chunks = docs
        .get(&ContentOrigin::TestEntityRust)
        .expect("Contains test data. qed");
    assert_eq!(dbg!(chunks).len(), 1);
    let chunk = &chunks[0];

    let suggestion_set =
        reflow(&ContentOrigin::TestEntityRust, chunk, &CONFIG).expect("Reflow is wokring. qed");

    let suggestion = suggestion_set
        .first()
        .expect("Contains one suggestion. qed");

    dbg!(load_span_from(&mut CONTENT.as_bytes(), suggestion.span).unwrap());

    let replacement = suggestion
        .replacements
        .first()
        .expect("There is a replacement. qed");
    assert_eq!(replacement.as_str(), EXPECTED);
}

#[test]
fn reflow_doc_indentations() {
    const CONTENT: &str = r##"
    #[doc = r#"A comment with indentation that spans over
                two lines and should be rewrapped.
            "#]
    struct Fluffy {};"##;

    const EXPECTED: &str = r##"A comment with indentation"#]
    #[doc = r#"that spans over two lines"#]
    #[doc = r#"and should be rewrapped."##;

    let docs = Documentation::load_from_str(ContentOrigin::TestEntityRust, CONTENT, true, false);
    assert_eq!(dbg!(&docs).entry_count(), 1);
    let chunks = docs
        .get(&ContentOrigin::TestEntityRust)
        .expect("Contains test data. qed");
    assert_eq!(dbg!(chunks).len(), 1);
    let chunk = &chunks[0];

    let cfg = ReflowConfig {
        max_line_length: 45,
    };
    let suggestion_set =
        reflow(&ContentOrigin::TestEntityRust, chunk, &cfg).expect("Reflow is working. qed");

    let suggestions = suggestion_set
        .first()
        .expect("Contains one suggestion. qed");

    let replacement = suggestions
        .replacements
        .first()
        .expect("There is a replacement. qed");
    assert_eq!(replacement.as_str(), EXPECTED);
}

#[test]
fn reflow_markdown() {
    reflow_fluff!(60 break ["Possible **ways** to run __rustc__ and request various parts of LTO.",
                        " `markdown` syntax which leads to __unbreakables__?  With emojis: 🚤w🌴x🌋y🍈z🍉0."] =>
        r#"Possible **ways** to run __rustc__ and request various
/// parts of LTO. `markdown` syntax which leads to
/// __unbreakables__? With emojis: 🚤w🌴x🌋y🍈z🍉0."#);
}

#[test]
fn reflow_two_paragraphs_not_required() {
    reflow_fluff!(80 break ["A short paragraph followed by another one.", "", "Surprise, we have another parapgrah."]
            => ok);
}

#[test]
fn reflow_fold_two_to_one() {
    reflow_fluff!(20 break ["A 🚤>", "<To 🌴/🍉&🍈"]
            => "A 🚤> <To 🌴/🍉&🍈");
}

#[test]
fn reflow_split_one_into_three() {
    reflow_fluff!(9 break ["A 🌴xX 🍉yY 🍈zZ"]
            => "A 🌴xX\n/// 🍉yY\n/// 🍈zZ");
}

#[test]
fn reflow_markdown_two_paragraphs() {
    const CONTENT: &str = "/// Possible __ways__ to run __rustc__ and request various parts of LTO.
///
/// Some more text after the newline which **represents** a paragraph";

    let expected = vec![
        r#"Possible __ways__ to run __rustc__ and request various
/// parts of LTO."#,
        r#"Some more text after the newline which **represents** a
/// paragraph"#,
    ];

    let _ = env_logger::Builder::new()
        .filter(None, log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    let docs = Documentation::load_from_str(ContentOrigin::TestEntityRust, CONTENT, true, false);
    assert_eq!(docs.entry_count(), 1);
    let chunks = docs
        .get(&ContentOrigin::TestEntityRust)
        .expect("Contains test data. qed");
    assert_eq!(dbg!(chunks).len(), 1);
    let chunk = &chunks[0];

    let cfg = ReflowConfig {
        max_line_length: 60,
    };

    let suggestion_set =
        reflow(&ContentOrigin::TestEntityRust, chunk, &cfg).expect("Reflow is working. qed");

    for (sug, expected) in suggestion_set.iter().zip(expected) {
        assert_eq!(sug.replacements.len(), 1);
        let replacement = sug
            .replacements
            .first()
            .expect("An replacement exists. qed");

        assert_eq!(replacement.as_str(), expected);
    }
}

#[test]
fn reflow_markdown_two_paragraphs_doc() {
    let chyrped = chyrp_up!(
        r#"A long comment that spans over two lines.

With a second part that is fine"#
    );
    println!("{chyrped}");

    let expected = vec![
        r##"A long comment that spans over"#]
#[doc=r#"two lines."##,
        r#"With a second part that is fine"#,
    ];

    let docs = Documentation::load_from_str(ContentOrigin::TestEntityRust, chyrped, true, false);
    assert_eq!(docs.entry_count(), 1);
    let chunks = docs
        .get(&ContentOrigin::TestEntityRust)
        .expect("Contains test data. qed");

    let cfg = ReflowConfig {
        max_line_length: 45,
    };

    for (chunk, expect) in chunks.iter().zip(expected) {
        let suggestion_set =
            reflow(&ContentOrigin::TestEntityRust, chunk, &cfg).expect("Reflow is working. qed");
        let sug = suggestion_set.first().expect("Contains a suggestion. qed");
        let replacement = sug
            .replacements
            .first()
            .expect("An replacement exists. qed");
        assert_eq!(replacement.as_str(), expect);
    }
}

#[test]
fn reflow_doc_short() {
    reflow_chyrp!(40 break ["a", "b", "c"] => r#"a b c"#);
}

#[test]
fn reflow_doc_indent_middle() {
    reflow_chyrp!(29 break ["First line", "     Second line", "         third line"]
        => r##"First line Second"#]
#[doc=r#"line third line"##);
}

#[test]
fn reflow_doc_long() {
    reflow_chyrp!(40 break ["One line which is quite long and needs to be reflown in another line."]
        => r##"One line which is quite long"#]
#[doc=r#"and needs to be reflown in"#]
#[doc=r#"another line."##);
}

#[test]
fn reflow_sole_markdown() {
    const CONFIG: ReflowConfig = ReflowConfig {
        max_line_length: 60,
    };

    const CONTENT: &str = "# Possible __ways__ to run __rustc__ and request various parts of LTO.

A short line but long enough such that we reflow it. Yada lorem ipsum stuff needed.

- a list
- another point

Some <pre>Hmtl tags</pre>.

Some more text after the newline which **represents** a paragraph
in two lines. In my opinion paraghraphs are always multiline. Fullstop.";

    const EXPECTED: &[(&str, Span)] = &[
        (
            r#"A short line but long enough such that we reflow it. Yada
lorem ipsum stuff needed."#,
            Span {
                start: LineColumn { line: 3, column: 0 },
                end: LineColumn {
                    line: 3,
                    column: 83,
                },
            },
        ),
        (
            r#"Some more text after the newline which **represents** a
paragraph in two lines. In my opinion paraghraphs are always
multiline. Fullstop."#,
            Span {
                start: LineColumn {
                    line: 10,
                    column: 0,
                },
                end: LineColumn {
                    line: 11,
                    column: 70,
                },
            },
        ),
    ];

    let _ = env_logger::Builder::new()
        .filter(None, log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    let docs =
        Documentation::load_from_str(ContentOrigin::TestEntityCommonMark, CONTENT, true, false);
    assert_eq!(docs.entry_count(), 1);
    let chunks = docs
        .get(&ContentOrigin::TestEntityCommonMark)
        .expect("Contains test data. qed");
    assert_eq!(dbg!(chunks).len(), 1);
    let chunk = chunks.first().unwrap();

    let suggestion_set = reflow(&ContentOrigin::TestEntityCommonMark, chunk, &CONFIG)
        .expect("Reflow is working. qed");
    assert_eq!(suggestion_set.len(), 2);

    for (sug, &(expected_content, expected_span)) in suggestion_set.iter().zip(EXPECTED.iter()) {
        dbg!(&sug.span);
        dbg!(&sug.range);
        assert_eq!(sug.replacements.len(), 1);
        let replacement = sug
            .replacements
            .first()
            .expect("Reflow always provides a replacement string. qed");

        assert_eq!(sug.span, expected_span);

        assert_eq!(replacement.as_str(), expected_content);
    }
}

#[test]
fn reflow_line_delimiters() {
    const TEST_DATA: &[(&str, &str)] = &[
        ("Two lines\nhere", "\n"),
        ("Two lines\r\nhere", "\r\n"),
        ("\r\n\r\n", "\r\n"),
        ("\n\r\n\r\n", "\n\r"),
        ("\n\n\n\r\n", "\n"),
        ("\n\r\n\n\r\n", "\n\r"),
        ("Two lines\n\rhere", "\n\r"),
        ("Two lines\nhere\r\nand more\r\nsfd", "\r\n"),
        ("Two lines\r\nhere\nand more\n", "\n"),
        ("Two lines\nhere\r\nand more\n\r", "\n"),
        ("Two lines\nhere\r\nand more\n", "\n"),
    ];
    for (input, expected) in TEST_DATA {
        let expected = *expected;
        println!("{input:?} should detect {expected:?}");
        assert_eq!(extract_delimiter(input), Some(expected));
    }
}

#[test]
fn reflow_check_span() {
    const CONFIG: ReflowConfig = ReflowConfig {
        max_line_length: 27,
    };

    const CONTENT: &str = "/// A comment as we have many here and we will always
/// have.
struct Fff;
";

    const EXPECTED_REPLACEMENT: &[&str] =
        &["A comment as we have\n/// many here and we will\n/// always have."];

    const EXPECTED_SPAN: Span = Span {
        start: LineColumn { line: 1, column: 4 },
        end: LineColumn { line: 2, column: 8 },
    };

    let docs = Documentation::load_from_str(ContentOrigin::TestEntityRust, CONTENT, true, false);
    assert_eq!(docs.entry_count(), 1);
    let chunks = docs
        .get(&ContentOrigin::TestEntityRust)
        .expect("Contains test data. qed");
    assert_eq!(dbg!(chunks).len(), 1);
    let chunk = chunks.first().unwrap();

    let suggestion_set =
        reflow(&ContentOrigin::TestEntityRust, chunk, &CONFIG).expect("Reflow is working. qed");
    assert_eq!(suggestion_set.len(), 1);
    let suggestion = suggestion_set
        .first()
        .expect("Contains one suggestion. qed");

    assert_eq!(suggestion.span, EXPECTED_SPAN);
    assert_eq!(suggestion.replacements.as_slice(), EXPECTED_REPLACEMENT);
}

#[test]
fn reflow_readme() {
    // TODO reduce this to the minimal failing test case
    const README: &str = include_str!("../../README.md");

    reflow_content!(80usize break ContentOrigin::TestEntityCommonMark, README => ok);
}

#[test]
fn reflow_cmark_nested_link_types() {
    reflow_content!(80usize break ContentOrigin::TestEntityCommonMark, "[![yada](image_url)](link_url)" => ok);
}

#[test]
fn reflow_cmark_headlines() {
    reflow_content!(20usize break ContentOrigin::TestEntityCommonMark,
        r######"
# 🔒😑

It is too damn long 🐡.

```sh
shell
```

## 🥁 second

Yada
"###### => patches [ r#"It is too damn long
🐡.
"# ] );
}

const MINIFIED_README: &str = r###"# cargo-spellcheck

[![crates.io](https://img.source/cargo_spellcheck.svg)](https://crates.io)

<pre>
<font color="#CC0000"><b>error</b></font>
</pre>

## Continuous Integration / CI

`cargo spellcheck` can be configured with `-m <code>` to return a non-zero
return code if mistakes are found instead of `0`.

## Implemented Features + Roadmap

* [x] Parse doc comments from arbitrary files
* [x] Decent error printing

## Footy

Ref [^footnote]

## Table, no touch

|a                  |b               |c                          |d                                |
|-------------------|----------------|---------------------------|---------------------------------|
|0000000000000000000|1111111111111111|2                          |                                3|

## Quote

> I quote quoted quotes
> quoting quoted
> quotes, which
> shall remain untouched for now.

[footnote]: Down in the abyss.
"###;

#[test]
fn reflow_minified_readme_patches() {
    reflow_content!(30usize break ContentOrigin::TestEntityCommonMark, MINIFIED_README
    => patches [
r##"`cargo spellcheck` can be
configured with `-m <code>` to
return a non-zero return code
if mistakes are found instead
of `0`.
"##
    ]);
}

#[test]
fn reflow_minified_readme_applied() {
    reflow_content!(30usize break ContentOrigin::TestEntityCommonMark, MINIFIED_README
        => applied
        r###"# cargo-spellcheck

[![crates.io](https://img.source/cargo_spellcheck.svg)](https://crates.io)

<pre>
<font color="#CC0000"><b>error</b></font>
</pre>

## Continuous Integration / CI

`cargo spellcheck` can be
configured with `-m <code>` to
return a non-zero return code
if mistakes are found instead
of `0`.

## Implemented Features + Roadmap

* [x] Parse doc comments from arbitrary files
* [x] Decent error printing

## Footy

Ref [^footnote]

## Table, no touch

|a                  |b               |c                          |d                                |
|-------------------|----------------|---------------------------|---------------------------------|
|0000000000000000000|1111111111111111|2                          |                                3|

## Quote

> I quote quoted quotes
> quoting quoted
> quotes, which
> shall remain untouched for now.

[footnote]: Down in the abyss.
"###
    );
}

fn assert_only_line_delimiter(source: &str, delimiter: &str) {
    assert!(
        source.contains(delimiter),
        "expected newlines in {source:?}"
    );
    let without_delimiters = source.replace(delimiter, "");
    assert!(
        !without_delimiters.contains(['\r', '\n']),
        "mixed delimiters: {source:?}"
    );
}

fn assert_rust_reflow_preserves_prose(source: &str) -> String {
    let before = rust_doc_words(source);
    let (wrapped, count) = apply_rust_reflow(source, 40);
    assert_eq!(count, 1);
    assert_ne!(wrapped, source);
    assert!(wrapped.lines().all(|line| line.chars().count() <= 40));
    assert_eq!(rust_doc_words(&wrapped), before);
    assert_eq!(apply_rust_reflow(&wrapped, 40), (wrapped.clone(), 0));
    wrapped
}

#[test]
fn reflow_source_line_delimiter_single_line_docs() {
    for (prefix, suffix) in [("/// ", ""), ("#[doc=\"", "\"]"), ("#[doc=r#\"", "\"#]")] {
        let source = format!(
            "{prefix}Documentation contains enough ordinary words to require wrapping across several lines.{suffix}\r\nstruct Example;\r\n"
        );
        let wrapped = assert_rust_reflow_preserves_prose(&source);
        assert_only_line_delimiter(&wrapped, "\r\n");
        assert!(wrapped.ends_with("\r\nstruct Example;\r\n"));
    }
}

#[test]
fn reflow_source_line_delimiter_first_selected_literal_wins() {
    for (first, later) in [("\r\n", "\n"), ("\n", "\r\n")] {
        for (prefix, suffix) in [("/// ", ""), ("#[doc=\"", "\"]")] {
            let unrelated = format!("// Unrelated.{later}");
            let tail = format!("{later}struct Example;{later}");
            let source = format!(
                "{unrelated}{prefix}A short introduction.{suffix}{first}\
                 {prefix}Documentation continues with enough ordinary words to require wrapping.{suffix}{later}\
                 {prefix}More ordinary words also require wrapping across several lines.{suffix}{tail}"
            );
            let wrapped = assert_rust_reflow_preserves_prose(&source);
            let selected = wrapped
                .strip_prefix(&unrelated)
                .expect("unselected leading line must be untouched")
                .strip_suffix(&tail)
                .expect("unselected trailing lines must be untouched");
            assert_only_line_delimiter(selected, first);
        }
    }
}

#[test]
fn reflow_source_line_delimiter_conditional_skip_preserves_source() {
    for (first, later) in [("\r\n", "\r\n"), ("\r\n", "\n"), ("\n", "\r\n")] {
        let source = format!(
            "/// Shared introduction.{first}\
             #[cfg_attr(feature = \"one\", doc = \"Documentation contains enough ordinary words to require wrapping across several lines.\")]{later}\
             struct Example;{later}"
        );
        syn::parse_file(&source).unwrap();
        let origin = ContentOrigin::TestEntityRust;
        let docs = Documentation::load_from_str(origin.clone(), &source, true, false);
        let chunks = docs.get(&origin).unwrap();
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|chunk| !chunk.is_reflow_safe()));
        let checker = Reflow::new(ReflowConfig {
            max_line_length: 40,
        })
        .unwrap();
        let suggestions = checker.check(&origin, chunks).unwrap();
        assert!(suggestions.is_empty());
        assert_eq!(apply_reflow_patches(&source, &suggestions), source);
    }
}

#[test]
fn reflow_source_line_delimiter_commonmark_uses_original_content() {
    for delimiter in ["", "\n", "\r\n"] {
        let source = if delimiter.is_empty() {
            "Documentation contains enough ordinary words to require wrapping across several lines."
                .to_owned()
        } else {
            format!(
                "Documentation contains enough ordinary words{delimiter}to require wrapping across several lines.{delimiter}"
            )
        };
        let origin = ContentOrigin::TestEntityCommonMark;
        let docs = Documentation::load_from_str(origin.clone(), &source, true, false);
        let chunks = docs.get(&origin).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].as_str(), source);
        assert_eq!(
            chunks[0].preferred_line_delimiter(),
            if delimiter.is_empty() {
                None
            } else {
                Some(delimiter)
            }
        );
        let checker = Reflow::new(ReflowConfig {
            max_line_length: 40,
        })
        .unwrap();
        let suggestions = checker.check(&origin, chunks).unwrap();
        assert_eq!(suggestions.len(), 1);
        let wrapped = apply_reflow_patches(&source, &suggestions);
        assert_only_line_delimiter(
            &wrapped,
            if delimiter.is_empty() {
                "\n"
            } else {
                delimiter
            },
        );
        assert!(wrapped.lines().all(|line| line.chars().count() <= 40));
        assert_eq!(
            wrapped.split_whitespace().collect::<Vec<_>>(),
            source.split_whitespace().collect::<Vec<_>>()
        );
        assert_eq!(wrapped.ends_with('\n'), !delimiter.is_empty());
        let docs = Documentation::load_from_str(origin.clone(), &wrapped, true, false);
        assert!(checker
            .check(&origin, docs.get(&origin).unwrap())
            .unwrap()
            .is_empty());
    }
}

#[test]
fn reflow_crlf() {
    const INPUT: &str = "        /// cargo spellcheck can be configured with `-m <code>` to return a non-zero return code.\r\n        struct Foo {}";
    println!("{INPUT:?}");
    reflow_content!(40usize break ContentOrigin::TestEntityRust, INPUT
    => patches [
        "cargo spellcheck can be\r\n        /// configured with `-m <code>`\r\n        /// to return a non-zero return\r\n        /// code."
    ]);
}
