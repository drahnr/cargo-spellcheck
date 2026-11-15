use super::Cached;
use crate::checker::cached::CachedValue;
use crate::errors::*;
use fs_err as fs;
use lazy_static::lazy_static;

#[cfg(feature = "nlprules")]
use nlprule::Rules;
use nlprule::Tokenizer;
use std::collections::{hash_map::Entry, HashMap};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use thousands::Separable;

#[cfg(test)]
mod tests;

static DEFAULT_TOKENIZER_BYTES: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/en_tokenizer.bin"));

#[cfg(feature = "nlprules")]
static DEFAULT_RULES_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/en_rules.bin"));

lazy_static! {
    static ref TOKENIZER: Mutex<HashMap<Option<PathBuf>, Arc<Tokenizer>>> =
        Mutex::new(HashMap::new());
}

fn maybe_display_micros(maybe_duration: impl Into<Option<std::time::Duration>>) -> String {
    maybe_duration
        .into()
        .map(|d| d.as_micros().separate_with_underscores())
        .unwrap_or_else(|| "-".to_owned())
}

pub fn project_dir() -> Result<directories::ProjectDirs> {
    directories::ProjectDirs::from("io", "ahoi", "cargo-spellcheck")
        .ok_or_else(|| color_eyre::eyre::eyre!("Missing project dir"))
}

fn tokenizer_inner<P: AsRef<Path>>(
    override_path: Option<P>,
    cache_dir: &Path,
) -> Result<Tokenizer> {
    log::info!("🧮 Loading tokenizer...");
    let tokenizer = if let Some(override_path) = override_path.as_ref() {
        let override_path = override_path.as_ref();
        let mut cached = Cached::new(override_path.display().to_string(), cache_dir)?;
        let CachedValue {
            fetch,
            update,
            creation,
            total,
            value: tokenizer,
        } = cached.fetch_or_update(|override_path| {
            let f = fs::File::open(override_path)?;
            Ok(Tokenizer::from_reader(f)?)
        })?;
        log::info!("🧮 Loaded tokenizer in {total} us (fetch: {fetch} us, update: {update} us, creation: {creation} us)",
            total = maybe_display_micros(total),
            fetch = maybe_display_micros(fetch),
            update = maybe_display_micros(update),
            creation = maybe_display_micros(creation),
        );
        tokenizer
    } else {
        let total_start = std::time::Instant::now();
        let tokenizer = Tokenizer::from_reader(&mut &*DEFAULT_TOKENIZER_BYTES)?;
        log::info!(
            "🧮 Loaded (builtin) tokenizer in {} us",
            maybe_display_micros(total_start.elapsed())
        );
        tokenizer
    };
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
            let tokenizer = tokenizer_inner(override_path, project_dir()?.cache_dir())?;
            let tokenizer = Arc::new(tokenizer);
            empty.insert(tokenizer.clone());
            Ok(tokenizer)
        }
    }
}

#[cfg(feature = "nlprules")]
lazy_static! {
    static ref RULES: Mutex<HashMap<Option<PathBuf>, Arc<Rules>>> = Mutex::new(HashMap::new());
}

#[cfg(feature = "nlprules")]
fn rules_inner<P: AsRef<Path>>(override_path: Option<P>, cache_dir: &Path) -> Result<Rules> {
    log::info!("🧮 Loading rules...");
    let rules = if let Some(override_path) = override_path.as_ref() {
        let override_path = override_path.as_ref();
        let mut cached = Cached::new(override_path.display().to_string(), cache_dir)?;
        let CachedValue {
            fetch,
            update,
            creation,
            total,
            value: rules,
        } = cached.fetch_or_update(|override_path| {
            let f = fs::File::open(override_path)?;
            Ok(Rules::from_reader(f)?)
        })?;
        log::info!("🧮 Loaded rules in {total} us (fetch: {fetch} us, update: {update} us, creation: {creation} us)",
            total = maybe_display_micros(total),
            fetch = maybe_display_micros(fetch),
            update = maybe_display_micros(update),
            creation = maybe_display_micros(creation),
        );
        rules
    } else {
        // there is no speedgain for the builtin
        let total_start = std::time::Instant::now();
        let rules = Rules::from_reader(&mut &*DEFAULT_RULES_BYTES)?;
        log::info!(
            "🧮 Loaded (builtin) rules in {} us",
            maybe_display_micros(total_start.elapsed())
        );
        rules
    };

    Ok(rules)
}

#[cfg(feature = "nlprules")]
pub(crate) fn rules<P: AsRef<Path> + Clone>(override_path: Option<P>) -> Result<Arc<Rules>> {
    match RULES
        .lock()
        .unwrap()
        .entry(override_path.clone().map(|p| p.as_ref().to_path_buf()))
    {
        Entry::Occupied(occupied) => Ok(occupied.get().clone()),
        Entry::Vacant(empty) => {
            let rules = rules_inner(override_path, project_dir()?.cache_dir())?;
            let rules = Arc::new(rules);
            empty.insert(rules.clone());
            Ok(rules)
        }
    }
}

#[cfg(any(test, feature = "hunspell", feature = "spellbook", feature = "zet"))]
use crate::Range;

#[cfg(any(test, feature = "hunspell", feature = "spellbook", feature = "zet"))]
pub(crate) fn apply_tokenizer<'t, 'z>(
    tokenizer: &'t Arc<Tokenizer>,
    text: &'z str,
) -> impl std::iter::Iterator<Item = Range> + 'z
where
    't: 'z,
{
    tokenizer.pipe(text).flat_map(|sentence| {
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
            let char_range = token.span().char().clone();

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
            let belongs_to_genitive_s = !matches!(s, "(" | ")" | r#"""#);
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
                        acc.append(&mut backlog);
                        acc.push(char_range);
                        Stage::Empty
                    }
                }
            } else {
                acc.append(&mut backlog);
                acc.push(char_range);
                Stage::Empty
            };
        }
        acc.append(&mut backlog);
        acc.into_iter()
    })
}
