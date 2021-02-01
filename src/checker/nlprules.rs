//! A dictionary check with affixes, backed by `libhunspell`
//!
//! Does not check grammar, but tokenizes the documentation chunk,
//! and checks the individual tokens against the dictionary using
//! the defined affixes.
//! Can handle multiple dictionaries.

use super::{Checker, Detector, Documentation, Suggestion, SuggestionSet};

use log::{debug, trace};
use rayon::prelude::*;

use anyhow::Result;

use nlprule::{Tokenizer, Rules};
use nlprule::types::Suggestion as NlpFix;

static TOKENIZER_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tokenizer.bin"));
static RULES_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/rules.bin"));

lazy_static::lazy_static!{
    static ref TOKENIZER: Tokenizer = Tokenizer::new_from(&mut &*TOKENIZER_BYTES).unwrap();
    static ref RULES: Rules = Rules::new_from(&mut &*RULES_BYTES).unwrap();
}

pub(crate) struct NlpRulesChecker;

impl Checker for NlpRulesChecker {
    type Config = ();
    fn check<'a, 's>(docu: &'a Documentation, _config: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's,
    {
        let suggestions = docu
            .par_iter()
            .try_fold::<SuggestionSet, Result<_>, _, _>(
                || SuggestionSet::new(),
                move |mut acc, (origin, chunks)| {
                    debug!("Processing {}", origin.as_path().display());

                    'check: for chunk in chunks {
                        let plain = chunk.erase_cmark();
                        trace!("{:?}", &plain);
                        let txt = plain.as_str();
                        let nlpfixes = RULES.suggest(txt, &TOKENIZER);
                        if nlpfixes.is_empty() {
                            continue 'check;
                        }
                        'nlp: for NlpFix { message, start, end, replacements, .. } in nlpfixes {
                            if start > end  {
                                continue 'nlp;
                            }
                            let range = start..(end+1);
                            for (range, span) in plain.find_spans(range) {
                                acc.add(
                                    origin.clone(),
                                    Suggestion {
                                        detector: Detector::NlpRules,
                                        range,
                                        span,
                                        origin: origin.clone(),
                                        replacements:
                                            replacements
                                            .iter()
                                            .map(|x| x.clone())
                                            .collect(),
                                        chunk,
                                        description: Some(message.clone()),
                                    },
                                );
                            }
                        }
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

        // TODO sort spans by file and line + column
        Ok(suggestions)
    }
}
