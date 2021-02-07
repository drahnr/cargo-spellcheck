//! A nlp based rule checker base on `nlprule`
//!
//! Does check grammar, and is supposed to only check for grammar.
//! Sentence splitting is done in hand-waving way. To be improved.

use super::{Checker, Detector, Documentation, Suggestion, SuggestionSet};
use crate::{CheckableChunk, ContentOrigin};

use log::{debug, trace};
use rayon::prelude::*;

use anyhow::Result;

use nlprule::types::Suggestion as NlpFix;
use nlprule::{Rules, Tokenizer};

static TOKENIZER_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tokenizer.bin"));
static RULES_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/rules.bin"));

lazy_static::lazy_static! {
    static ref TOKENIZER: Tokenizer = {
        Tokenizer::from_reader(&mut &*TOKENIZER_BYTES)
            .expect("build.rs pulls valid tokenizer description. qed")
    };
    static ref RULES: Rules = {
        let rules = Rules::from_reader(&mut &*RULES_BYTES)
            .expect("build.rs pulls valid rules description. qed")
            .into_iter()
            .filter(|rule| {
                match rule.category_id().to_lowercase().as_ref() {
                    // The hunspell backend is aware of
                    // custom lingo, which this one is not,
                    // so there would be a lot of false
                    // positives.
                    "misspelling"  => false,
                    // Anything quotes related is not relevant
                    // for code documentation.
                    "typography" => false,
                    other => true,
                }

            })
            .collect::<Rules>();
        rules
    };
}

pub(crate) struct NlpRulesChecker;

impl Checker for NlpRulesChecker {
    type Config = ();
    fn check<'a, 's>(docu: &'a Documentation, _config: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's,
    {
        // avoid poisioned `Once` calls inside the parallelized iterators
        let tokenizer = &*TOKENIZER;
        let rules = &*RULES;

        let suggestions = docu
            .par_iter()
            .try_fold::<SuggestionSet, Result<_>, _, _>(
                || SuggestionSet::new(),
                move |mut acc, (origin, chunks)| {
                    debug!("Processing {}", origin.as_path().display());

                    for chunk in chunks {
                        acc.extend(
                            origin.clone(),
                            check_sentence(origin.clone(), chunk, tokenizer, rules),
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

/// Check one segmented sentence
fn check_sentence<'a>(
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
