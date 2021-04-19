//! A NLP based rule checker base on `nlprule`
//!
//! Does check grammar, and is supposed to only check for grammar.
//! Sentence splitting is done in hand-waving way. To be improved.

use super::{Checker, Detector, Documentation, Suggestion, SuggestionSet};
use crate::{CheckableChunk, ContentOrigin};

use crate::errors::*;
use log::{debug, trace, warn};
use rayon::prelude::*;

use nlprule::{Rules, Tokenizer};

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
        let tokenizer = super::tokenizer(config.override_tokenizer.as_ref())?;
        let rules = super::rules(config.override_tokenizer.as_ref())?;

        let rules = rules
            .into_iter()
            .filter(|rule| {
                match rule
                    .category_type()
                    .map(str::to_lowercase)
                    .as_ref()
                    .map(|x| x as &str)
                {
                    // The hunspell backend is aware of
                    // custom lingo, which this one is not,
                    // so there would be a lot of false
                    // positives.
                    Some("misspelling") => false,
                    // Anything quotes related is not relevant
                    // for code documentation.
                    Some("typographical") => false,
                    _other => true,
                }
            })
            .collect::<Rules>();

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

    'nlp: for fix in nlpfixes {
        let message = fix.message();
        let replacements = fix.replacements();
        let start = fix.span().char().start;
        let end = fix.span().char().end;
        if start > end {
            warn!("BUG: crate nlprule yielded a negative range {:?} for chunk in {}, please file a bug", start..end, &origin);
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
                    description: Some(message.to_owned()),
                }),
        );
    }

    acc
}
