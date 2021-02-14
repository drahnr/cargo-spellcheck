//! A nlp based rule checker base on `nlprule`
//!
//! Does check grammar, and is supposed to only check for grammar.
//! Sentence splitting is done in hand-waving way. To be improved.

use super::{Checker, Detector, Documentation, Suggestion, SuggestionSet};
use crate::{CheckableChunk, ContentOrigin};

use anyhow::Result;
use fs_err as fs;
use log::{debug, info, trace, warn};
use rayon::prelude::*;

use nlprule::types::Suggestion as NlpFix;
use nlprule::{Rules, Tokenizer};

static DEFAULT_TOKENIZER_BYTES: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/en_tokenizer.bin"));
static DEFAULT_RULES_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/en_rules.bin"));

pub(crate) struct NlpRulesChecker;

impl Checker for NlpRulesChecker {
    type Config = crate::config::NlpRulesConfig;

    fn detector() -> Detector {
        Detector::NlpRules
    }

    fn check<'a, 's>(docu: &'a Documentation, config: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's,
    {
        info!("Loading tokenizer...");

        let tokenizer = config.override_tokenizer.as_ref().map_or_else(
            || {
                Ok(Tokenizer::from_reader(&mut &*DEFAULT_TOKENIZER_BYTES)
                    .expect("build.rs pulls valid tokenizer description. qed"))
            },
            |path| -> Result<Tokenizer> {
                let f = fs::File::open(&path)?;
                Ok(Tokenizer::from_reader(f)?)
            },
        )?;

        info!("Loaded tokenizer.");

        info!("Loading rules..");

        let rules = config.override_rules.as_ref().map_or_else(
            || {
                Ok(Rules::from_reader(&mut &*DEFAULT_RULES_BYTES)
                    .expect("build.rs pulls valid rules set. qed"))
            },
            |path| -> Result<Rules> {
                let f = fs::File::open(&path)?;
                Ok(Rules::from_reader(f)?)
            },
        )?;

        let rules = rules
            .into_iter()
            .filter(|rule| {
                match rule.category_id().to_lowercase().as_ref() {
                    // The hunspell backend is aware of
                    // custom lingo, which this one is not,
                    // so there would be a lot of false
                    // positives.
                    "misspelling" => false,
                    // Anything quotes related is not relevant
                    // for code documentation.
                    "typography" => false,
                    _other => true,
                }
            })
            .collect::<Rules>();

        info!("Loaded rules.");

        let rules = &rules;
        let tokenizer = &tokenizer;
        let suggestions = docu
            .par_iter()
            .try_fold::<SuggestionSet, Result<_>, _, _>(
                || SuggestionSet::new(),
                move |mut acc, (origin, chunks)| {
                    debug!("Processing {}", origin.as_path().display());

                    for chunk in chunks {
                        acc.extend(
                            origin.clone(),
                            check_chunk(origin.clone(), chunk, tokenizer, rules),
                        );
                    }
                    Ok(acc)
                },
            )
            .try_reduce(
                || SuggestionSet::new(),
                |mut a, b| {
                    a.join(b);
                    Ok(a)
                },
            )?;

        Ok(suggestions)
    }
}

/// Check the plain text contained in chunk,
/// which can be one or more sentences.
fn check_chunk<'a>(
    origin: ContentOrigin,
    chunk: &'a CheckableChunk,
    tokenizer: &Tokenizer,
    rules: &Rules,
) -> Vec<Suggestion<'a>> {
    let plain = chunk.erase_cmark();
    trace!("{:?}", &plain);
    let txt = plain.as_str();

    let mut acc = Vec::with_capacity(32);

    let nlpfixes = rules.suggest(txt, tokenizer);
    if nlpfixes.is_empty() {
        return Vec::new();
    }

    'nlp: for NlpFix {
        message,
        start,
        end,
        replacements,
        ..
    } in nlpfixes
    {
        if start > end {
            warn!("BUG: crate nlprule yielded a negative range, please file a bug");
            continue 'nlp;
        }
        let range = start..end;
        acc.extend(
            plain
                .find_spans(range)
                .into_iter()
                .map(|(range, span)| Suggestion {
                    detector: Detector::NlpRules,
                    range,
                    span,
                    origin: origin.clone(),
                    replacements: replacements.iter().map(|x| x.clone()).collect(),
                    chunk,
                    description: Some(message.clone()),
                }),
        );
    }

    acc
}
