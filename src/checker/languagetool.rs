use super::*;

use crate::literalset::Range;
use languagetool_rs::{LanguageTool, Request};
pub struct LanguageToolChecker;

impl Checker for LanguageToolChecker {
    type Config = crate::config::LanguageToolConfig;
    fn check<'a, 's>(docu: &'a Documentation, config: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's,
    {
        let lt = LanguageTool::new(config.url.as_str())?;
        let suggestions = docu.iter().try_fold::<Vec<Suggestion>, _, Result<_>>(
            SuggestionSet::new(),
            |mut acc, (path, literal_sets)| {
                for cls in literal_sets {
                    let plain = cls.erase_markdown();
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
                                log::trace!("item.rule: {:?}", rule);
                            }
                            log::trace!("item.context: {:?}", item.context);
                            log::trace!("item.message: {:?}", item.message);
                            log::trace!("item.short_message: {:?}", item.short_message);
                            // TODO convert response to offsets and errors with the matching literal
                            for (literal, span) in plain.linear_range_to_spans(Range {
                                start: item.offset as usize,
                                end: (item.offset + item.length) as usize,
                            }) {
                                acc.insert(path.to_owned(), Suggestion {
                                    detector: Detector::LanguageTool,
                                    span: span,
                                    path: PathBuf::from(path),
                                    replacements: item
                                        .replacements
                                        .iter()
                                        .filter_map(|x| x.value.clone())
                                        .collect(),
                                    literal: literal.into(),
                                    description: Some(item.message.clone()),
                                });
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
