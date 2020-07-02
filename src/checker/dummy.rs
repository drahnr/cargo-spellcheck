use super::tokenize;
use super::Checker;
use crate::documentation::Documentation;
use crate::suggestion::{Detector, Suggestion, SuggestionSet};
use anyhow::Result;
use log::trace;

/// A test checker that tokenizes and marks everything as wrong
pub struct DummyChecker;

impl Checker for DummyChecker {
    type Config = ();

    fn check<'a, 's>(docu: &'a Documentation, _: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's,
    {
        let suggestions = docu.iter().try_fold::<SuggestionSet, _, Result<_>>(
            SuggestionSet::new(),
            |mut acc, (origin, chunks)| {
                let chunk = chunks.iter().next().unwrap();
                let plain = chunk.erase_markdown();
                for (index, range) in dbg!(tokenize(plain.as_str())).into_iter().enumerate() {
                    trace!("Token: >{}<", &plain.as_str()[range.clone()]);
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
                            origin: origin.clone(),
                            replacements,
                            chunk,
                            description: None,
                        };
                        acc.add(origin.clone(), dbg!(suggestion));
                    }
                }
                Ok(acc)
            },
        )?;

        Ok(suggestions)
    }
}
