//! Everything is wrong, so wrong, even if it's correct.
//!
//! A test checker, only available for unit tests.

// use super::tokenize;
use super::{apply_tokenizer, Checker};
use crate::documentation::Documentation;
use crate::suggestion::{Detector, Suggestion, SuggestionSet};
use crate::util::sub_chars;
use anyhow::Result;
use log::trace;

/// A test checker that tokenizes and marks everything as wrong
pub struct DummyChecker;

impl Checker for DummyChecker {
    type Config = ();

    fn detector() -> Detector {
        Detector::Dummy
    }

    fn check<'a, 's>(docu: &'a Documentation, _: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's,
    {
        let tokenizer = super::tokenizer::<&std::path::PathBuf>(None)?;

        let suggestions = docu.iter().try_fold::<SuggestionSet, _, Result<_>>(
            SuggestionSet::new(),
            |mut acc, (origin, chunks)| {
                let chunk = chunks
                    .iter()
                    .next()
                    .expect("DummyChecker expects at least one chunk");
                let plain = chunk.erase_cmark();
                let txt = plain.as_str();
                for (index, range) in apply_tokenizer(&tokenizer, txt).enumerate() {
                    trace!("****Token[{}]: >{}<", index, sub_chars(txt, range.clone()));
                    let detector = Detector::Dummy;
                    let range2span = plain.find_spans(range.clone());
                    for (range, span) in range2span {
                        trace!(
                            "Suggestion for {:?} -> {}",
                            range,
                            chunk.display(range.clone())
                        );
                        let replacements = vec![format!("replacement_{}", index)];
                        let suggestion = Suggestion {
                            detector,
                            span,
                            range,
                            origin: origin.clone(),
                            replacements,
                            chunk,
                            description: None,
                        };
                        acc.add(origin.clone(), suggestion);
                    }
                }
                Ok(acc)
            },
        )?;

        Ok(suggestions)
    }
}
