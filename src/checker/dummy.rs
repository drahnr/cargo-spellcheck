//! Everything is wrong, so wrong, even if it's correct.
//!
//! A test checker, only available for unit tests.

// use super::tokenize;
use super::{apply_tokenizer, Checker};

use crate::suggestion::{Detector, Suggestion};
use crate::util::sub_chars;
use crate::{errors::*, CheckableChunk, ContentOrigin};

/// A test checker that tokenizes and marks everything as wrong
pub struct DummyChecker;

impl DummyChecker {
    pub fn new(_config: &<Self as Checker>::Config) -> Result<Self> {
        Ok(Self)
    }
}

impl Checker for DummyChecker {
    type Config = ();

    fn detector() -> Detector {
        Detector::Dummy
    }

    fn check<'a, 's>(
        &self,
        origin: &ContentOrigin,
        chunks: &'a [CheckableChunk],
    ) -> Result<Vec<Suggestion<'s>>>
    where
        'a: 's,
    {
        let tokenizer = super::tokenizer::<&std::path::PathBuf>(None)?;

        let mut acc = Vec::with_capacity(chunks.len());
        let chunk = chunks
            .first()
            .expect("DummyChecker expects at least one chunk");
        let plain = chunk.erase_cmark();
        let txt = plain.as_str();
        for (index, range) in apply_tokenizer(&tokenizer, txt).enumerate() {
            log::trace!("****Token[{}]: >{}<", index, sub_chars(txt, range.clone()));
            let detector = Detector::Dummy;
            let range2span = plain.find_spans(range.clone());
            for (range, span) in range2span {
                log::trace!(
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
                acc.push(suggestion);
            }
        }
        Ok(acc)
    }
}
