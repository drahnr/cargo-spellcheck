use anyhow::Result;
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
    info!("Loading tokenizer...");
    let tokenizer = if let Some(path) = override_path.as_ref() {
        let f = fs::File::open(path.as_ref())?;
        Tokenizer::from_reader(f)
    } else {
        Tokenizer::from_reader(&mut &*DEFAULT_TOKENIZER_BYTES)
    }?;
    info!("Loaded tokenizer.");
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

fn rules_inner<P: AsRef<Path>>(override_path: Option<P>) -> Result<Rules> {
    info!("Loading rules...");
    let rules = if let Some(override_path) = override_path.as_ref() {
        let f = fs::File::open(override_path.as_ref())?;
        Rules::from_reader(f)
    } else {
        Rules::from_reader(&mut &*DEFAULT_RULES_BYTES)
    }?;
    info!("Loaded rules.");
    Ok(rules)
}

pub(crate) fn rules<P: AsRef<Path> + Clone>(override_path: Option<P>) -> Result<Rules> {
    // XXX TODO right now `Rules` is not copy and only used in one place
    // so this is fine for now
    rules_inner(override_path)
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
            sentence.into_iter().filter_map(|token| {
                let range = token.char_span.0..token.char_span.1;
                if range.is_empty() {
                    None
                } else {
                    Some((token, range))
                }
            })
        })
        .flatten()
        .map(move |(token, range)| {
            // TODO XXX FIXME
            // Get rid of this asap once
            // https://github.com/bminixhofer/nlprule/issues/53
            // is resolved
            let offset_bytes =
                (token.sentence.as_ptr() as usize).saturating_sub(text.as_ptr() as usize);
            let offset_chars = text[..offset_bytes].chars().count();
            Range {
                start: range.start + offset_chars,
                end: range.end + offset_chars,
            }
        })
}
