use crate::errors::*;
use fs_err as fs;
use lazy_static::lazy_static;
use log::info;
use nlprule::{Rules, Tokenizer};
use std::collections::{hash_map::Entry, HashMap};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

static DEFAULT_TOKENIZER_BYTES: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/en_tokenizer.bin"));

static DEFAULT_RULES_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/en_rules.bin"));

lazy_static! {
    static ref TOKENIZER: Mutex<HashMap<Option<PathBuf>, Arc<Tokenizer>>> =
        Mutex::new(HashMap::new());
}

fn tokenizer_inner<P: AsRef<Path>>(override_path: Option<P>) -> Result<Tokenizer> {
    info!("🧮 Loading tokenizer...");
    let tokenizer = if let Some(path) = override_path.as_ref() {
        let f = fs::File::open(path.as_ref())?;
        Tokenizer::from_reader(f)
    } else {
        Tokenizer::from_reader(&mut &*DEFAULT_TOKENIZER_BYTES)
    }?;
    info!("🧮 Loaded tokenizer.");
    Ok(tokenizer)
}

pub(crate) fn tokenizer<P: AsRef<Path> + Clone>(
    override_path: Option<P>,
) -> Result<Arc<Tokenizer>> {
    match TOKENIZER
        .lock()
        .unwrap()
        .entry(override_path.clone().map(|x| x.as_ref().to_path_buf()))
    {
        Entry::Occupied(occupied) => Ok(occupied.get().clone()),
        Entry::Vacant(empty) => {
            let tokenizer = tokenizer_inner(override_path)?;
            let tokenizer = Arc::new(tokenizer);
            empty.insert(tokenizer.clone());
            Ok(tokenizer)
        }
    }
}

lazy_static! {
    static ref RULES: Mutex<HashMap<Option<PathBuf>, Arc<Rules>>> = Mutex::new(HashMap::new());
}

fn rules_inner<P: AsRef<Path>>(override_path: Option<P>) -> Result<Rules> {
    info!("🧮 Loading rules...");
    let rules = if let Some(override_path) = override_path.as_ref() {
        let f = fs::File::open(override_path.as_ref())?;
        Rules::from_reader(f)
    } else {
        Rules::from_reader(&mut &*DEFAULT_RULES_BYTES)
    }?;
    info!("🧮 Loaded rules.");
    Ok(rules)
}

pub(crate) fn rules<P: AsRef<Path> + Clone>(override_path: Option<P>) -> Result<Arc<Rules>> {
    match RULES
        .lock()
        .unwrap()
        .entry(override_path.clone().map(|x| x.as_ref().to_path_buf()))
    {
        Entry::Occupied(occupied) => Ok(occupied.get().clone()),
        Entry::Vacant(empty) => {
            let rules = rules_inner(override_path)?;
            let rules = Arc::new(rules);
            empty.insert(rules.clone());
            Ok(rules)
        }
    }
}

use crate::Range;

pub(crate) fn apply_tokenizer<'t, 'z>(
    tokenizer: &'t Arc<Tokenizer>,
    text: &'z str,
) -> impl std::iter::Iterator<Item = Range> + 'z
where
    't: 'z,
{
    tokenizer
        .pipe(text)
        .into_iter()
        .map(|sentence| {
            let mut backlog: Vec<Range> = Vec::with_capacity(4);
            let mut acc = Vec::with_capacity(32);
            let mut iter = sentence
                .into_iter()
                .filter(|token| !token.span().char().is_empty())
                .peekable();

            #[derive(Clone, Copy, Debug)]
            enum Stage {
                Empty,
                Pre,
                Tick,
            }

            let mut stage = Stage::Empty;

            // special cases all abbreviated variants, i.e. `isn't` such
            // that the tokenizer treats them as a single word.
            //
            // Also allows i.e. `ink!'s` to be detected as a single
            // token.
            while let Some(token) = iter.next() {
                let char_range = dbg!(&token).span().char().clone();

                let space = iter
                    .peek()
                    .map(|upcoming| upcoming.has_space_before())
                    .unwrap_or(false);
                let s = token.word().as_str();
                // TODO workaround for a bug in srx
                // TODO that does not split `[7f` after `[` as expected
                if s.starts_with('[') && char_range.len() > 1 {
                    acc.push((char_range.start)..(char_range.start + 1));
                    acc.push((char_range.start + 1)..(char_range.end));
                    continue;
                }
                let belongs_to_genitive_s = match s {
                    "(" | ")" | r#"""# => false,
                    _ => true,
                };
                stage = if belongs_to_genitive_s {
                    match stage {
                        Stage::Empty if s != "'" && !space => {
                            backlog.push(char_range);
                            Stage::Pre
                        }
                        Stage::Pre if s != "'" && !space => {
                            backlog.push(char_range);
                            Stage::Pre
                        }
                        Stage::Pre if s == "'" && !space => {
                            backlog.push(char_range);
                            Stage::Tick
                        }
                        Stage::Tick if s != "'" => {
                            // combine all in backlog to one
                            acc.push(backlog.first().unwrap().start..char_range.end);
                            backlog.clear();
                            Stage::Empty
                        }
                        _stage => {
                            acc.extend(backlog.drain(..));
                            acc.push(char_range);
                            Stage::Empty
                        }
                    }
                } else {
                    acc.extend(backlog.drain(..));
                    acc.push(char_range);
                    Stage::Empty
                };
            }
            acc.extend(backlog.drain(..));
            acc.into_iter()
        })
        .flatten()
}

#[cfg(test)]
mod tests {
    use crate::sub_chars;

    use super::*;

    #[test]
    fn tokenize_for_abbrev_sentence() {
        let tok = tokenizer::<PathBuf>(None).unwrap();
        let ranges = apply_tokenizer(&tok, "It isn't that different.");

        ranges
            .zip([0_usize..2, 3..8, 9..13, 14..23].iter().cloned())
            .for_each(|(is, expect)| {
                assert_eq!(is, expect);
            });
    }

    #[test]
    fn tokenize_for_abbrev_short() {
        let tok = tokenizer::<PathBuf>(None).unwrap();
        let mut ranges = apply_tokenizer(&tok, "isn't");
        assert_eq!(ranges.next(), Some(0_usize..5));
    }

    #[test]
    fn tokenize_ink_bang_0_tick_s() {
        let tok = tokenizer::<PathBuf>(None).unwrap();
        let mut ranges = apply_tokenizer(&tok, "ink!'s");

        assert_eq!(ranges.next(), Some(0_usize..6));
    }

    #[test]
    fn tokenize_ink_bang_1_tick_s_w_brackets() {
        let tok = tokenizer::<PathBuf>(None).unwrap();
        let mut ranges = apply_tokenizer(&tok, "(ink!'s)");

        assert_eq!(ranges.next(), Some(0_usize..1));
        assert_eq!(ranges.next(), Some(1_usize..7));
        assert_eq!(ranges.next(), Some(7_usize..8));
    }

    #[test]
    fn tokenize_ink_bang_2_tick_s_w_brackets_spaced() {
        let tok = tokenizer::<PathBuf>(None).unwrap();
        let mut ranges = apply_tokenizer(&tok, "( ink!'s )");

        assert_eq!(ranges.next(), Some(0_usize..1));
        assert_eq!(ranges.next(), Some(2_usize..8));
        assert_eq!(ranges.next(), Some(9_usize..10));
    }

    #[test]
    fn tokenize_single_ticks_w_brackets() {
        let tok = tokenizer::<PathBuf>(None).unwrap();
        let ranges = apply_tokenizer(&tok, "the ('lock funds') transaction");

        ranges
            .zip([0_usize..3, 4..5, 5..6, 6..10, 11..16].iter().cloned())
            .for_each(|(is, expect)| {
                assert_eq!(is, expect);
            });
    }

    #[test]
    fn tokenize_double_ticks() {
        let tok = tokenizer::<PathBuf>(None).unwrap();
        let ranges = apply_tokenizer(&tok, r#"the "lock funds" transaction"#);

        ranges
            .zip(
                [0_usize..3, 4..5, 5..9, 10..15, 15..16, 17..28]
                    .iter()
                    .cloned(),
            )
            .for_each(|(is, expect)| {
                assert_eq!(is, expect);
            });
    }

    #[test]
    fn tokenize_bracketed_w_tick_s_inside() {
        let tok = tokenizer::<PathBuf>(None).unwrap();
        let ranges = apply_tokenizer(&tok, r#"the (Xyz's) do"#);

        ranges
            .zip([0_usize..3, 4..5, 5..10, 10..11, 12..14].iter().cloned())
            .for_each(|(is, expect)| {
                assert_eq!(is, expect);
            });
    }

    #[test]
    fn tokenize_boring_genetive_s() {
        let tok = tokenizer::<PathBuf>(None).unwrap();
        let ranges = apply_tokenizer(&tok, r#"The Y's car is yellow."#);

        ranges
            .zip([0_usize..3, 4..7, 8..11].iter().cloned())
            .for_each(|(is, expect)| {
                assert_eq!(is, expect);
            });
    }

    #[test]
    fn tokenize_foo_dot() {
        let tok = tokenizer::<PathBuf>(None).unwrap();
        let ranges = apply_tokenizer(&tok, r#"Foo."#);

        ranges
            .zip([0_usize..3, 3..4].iter().cloned())
            .for_each(|(is, expect)| {
                assert_eq!(is, expect);
            });
    }

    #[test]
    fn tokenize_foo() {
        let tok = tokenizer::<PathBuf>(None).unwrap();
        let ranges = apply_tokenizer(&tok, r#"foo"#);

        ranges
            .zip([0_usize..3].iter().cloned())
            .for_each(|(is, expect)| {
                assert_eq!(is, expect);
            });
    }

    #[test]
    fn tokenize_square_bracket_foo_square_bracket() {
        let text = r#"[1337]"#;
        let tok = tokenizer::<PathBuf>(None).unwrap();
        let ranges = apply_tokenizer(&tok, text);

        ranges
            .zip([0_usize..1, 1..5, 5..6].iter().cloned())
            .for_each(|(is, expect)| {
                dbg!((sub_chars(text, is.clone()), sub_chars(text, expect.clone())));
                assert_eq!(is, expect);
            });
    }
}
