//! A NLP based rule checker base on `nlprule`
//!
//! Does check grammar, and is supposed to only check for grammar.
//! Sentence splitting is done in hand-waving way. To be improved.

use super::{Checker, Detector, Documentation, Suggestion, SuggestionSet};
use crate::{CheckableChunk, ContentOrigin};

use crate::errors::*;
use log::{debug, trace};
use rayon::prelude::*;
use std::collections::{hash_map::Entry, HashMap};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use nlprule::{Rules, Tokenizer};

use lazy_static::lazy_static;

lazy_static! {
    static ref RULES: Mutex<HashMap<Option<PathBuf>, Arc<Rules>>> = Mutex::new(HashMap::new());
}

pub(crate) fn filtered_rules<P: AsRef<Path> + Clone>(
    override_path: Option<P>,
) -> Result<Arc<Rules>> {
    match RULES
        .lock()
        .unwrap()
        .entry(override_path.clone().map(|x| x.as_ref().to_path_buf()))
    {
        Entry::Occupied(occupied) => Ok(occupied.get().clone()),
        Entry::Vacant(empty) => {
            let rules = super::rules(override_path)?;
            let rules = rules
                .rules()
                .iter()
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
                .cloned()
                .collect::<Rules>();

            let rules = Arc::new(rules);
            empty.insert(rules.clone());
            Ok(rules)
        }
    }
}

pub(crate) struct NlpRulesChecker {
    tokenizer: Arc<Tokenizer>,
    rules: Arc<Rules>,
}

impl NlpRulesChecker {
    pub fn new(config: &<Self as Checker>::Config) -> Result<Self> {
        let tokenizer = super::tokenizer(config.override_tokenizer.as_ref())?;
        let rules = filtered_rules(config.override_tokenizer.as_ref())?;
        Ok(Self { tokenizer, rules })
    }
}

impl Checker for NlpRulesChecker {
    type Config = crate::config::NlpRulesConfig;

    fn detector() -> Detector {
        Detector::NlpRules
    }

    fn check<'a, 's>(
        &self,
        origin: &ContentOrigin,
        chunks: &'a [CheckableChunk],
    ) -> Result<Vec<Suggestion<'s>>>
    where
        'a: 's,
    {
        let mut acc = Vec::with_capacity(chunks.len());

        for chunk in chunks {
            acc.extend(check_chunk(
                origin.clone(),
                chunk,
                &self.tokenizer,
                &self.rules,
            ));
        }

        Ok(acc)
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
            debug!("BUG: crate nlprule yielded a negative range {:?} for chunk in {}, please file a bug", start..end, &origin);
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
