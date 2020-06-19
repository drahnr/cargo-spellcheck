use crate::suggestion::{Detector, SuggestionSet,Suggestion};
use crate::config::Config;
use crate::documentation::Documentation;
use std::path::PathBuf;
use std::path::Path;
use super::Checker;
use anyhow::{anyhow, Result, Error};
use log::{trace,warn,info};
use super::tokenize;

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
                for range in tokenize(plain.as_str()) {
                    let detector = Detector::Dummy;
                    trace!("Range = {:?}", &range);
                    for (index, (literal, span)) in plain
                        .linear_range_to_spans(range.clone())
                        .into_iter()
                        .enumerate()
                    {
                        trace!("Span    {:?}", &span);
                        trace!("literal {:?}", &literal);
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
