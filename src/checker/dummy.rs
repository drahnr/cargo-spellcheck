use super::tokenize;
use super::Checker;
use crate::documentation::Documentation;
use crate::suggestion::{Detector, Suggestion, SuggestionSet};
use anyhow::{anyhow, Error, Result};
use log::{info, trace, warn};
use std::path::PathBuf;

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
            |mut acc, (path, literal_sets)| {
                let plain = literal_sets.iter().next().unwrap().erase_markdown();
                for (index, range) in tokenize(plain.as_str()).into_iter().enumerate() {
                    let detector = Detector::Dummy;
                    trace!("Range = {:?}", &range);
                    for (literal, span) in plain.linear_range_to_spans(range.clone()) {
                        trace!(
                            "literal pre {}, post {}, len {}",
                            literal.pre,
                            literal.post,
                            literal.len
                        );
                        trace!("index {}", index);
                        let replacements = vec![format!("replacement_{}", index)];
                        let suggestion = Suggestion {
                            detector,
                            span,
                            path: PathBuf::from(path),
                            replacements,
                            literal: literal.into(),
                            description: None,
                        };
                        acc.add(PathBuf::from(path), suggestion);
                    }
                }
                Ok(acc)
            },
        )?;

        Ok(suggestions)
    }
}
