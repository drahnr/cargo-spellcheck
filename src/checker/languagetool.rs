//! Check spelling and grammar by utilizing a languagetool server
//!
//! Can either be local or hosted.
//! Note that this does not do any rate limiting and will exhaust the
//! checks per second very quickly.

use super::*;

use crate::Range;
use languagetool_rs::{LanguageTool, Request};
use log::trace;

pub struct LanguageToolChecker;

impl Checker for LanguageToolChecker {
    type Config = crate::config::LanguageToolConfig;
    fn check<'a, 's>(docu: &'a Documentation, config: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's,
    {
        let lt = LanguageTool::new(config.url.as_str())?;
        let suggestions = docu.iter().try_fold::<SuggestionSet, _, Result<_>>(
            SuggestionSet::new(),
            |mut acc, (origin, chunks)| {
                for chunk in chunks {
                    let plain = chunk.erase_cmark();
                    log::trace!("markdown erasure: {:?}", &plain);
                    let req = Request::new(plain.to_string(), "en-US".to_owned());
                    let resp = lt.check(req)?;
                    if let Some(software) = resp.software {
                        log::trace!("sw: {:?}", software);
                    }
                    if let Some(matches) = resp.matches {
                        for item in matches {
                            if let Some(rule) = item.rule {
                                if rule.id == "EN_QUOTES" {
                                    // really annoying and pointless in code related documentation
                                    continue;
                                }
                                trace!("item.rule: {:?}", rule);
                            }
                            trace!("item.context: {:?}", item.context);
                            trace!("item.message: {:?}", item.message);
                            trace!("item.short_message: {:?}", item.short_message);
                            // TODO convert response to offsets and errors with the matching literal
                            for (range, span) in plain.find_spans(Range {
                                start: item.offset as usize,
                                end: (item.offset + item.length) as usize,
                            }) {
                                acc.add(
                                    origin.clone(),
                                    Suggestion {
                                        detector: Detector::LanguageTool,
                                        range,
                                        span,
                                        origin: origin.clone(),
                                        replacements: item
                                            .replacements
                                            .iter()
                                            .filter_map(|x| x.value.clone())
                                            .collect(),
                                        chunk: chunk,
                                        description: Some(item.message.clone()),
                                    },
                                );
                            }
                        }
                    }
                }
                Ok(acc)
            },
        )?;

        Ok(suggestions)
    }
}
