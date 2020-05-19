use super::*;

use languagetool_rs::{LanguageTool, Request};

pub struct LanguageToolChecker;

impl Checker for LanguageToolChecker {
    fn check<'a, 's>(docu: &'a Documentation) -> Result<Vec<Suggestion<'s>>>
    where
        'a: 's,
    {
        // let literals_to_string = |literals: &[ConsecutiveLiteralSet]| -> String {
        //     literals
        //         .into_iter()
        //         .map(|x| x.to_string())
        //         .collect::<Vec<String>>()
        //         .join("\n")
        // };

        // TODO make configurable
        // FIXME properly handle
        let url = "http://192.168.1.127:8010";
        let lt = LanguageTool::new(url)?;
        let suggestions = docu.iter().try_fold::<Vec<Suggestion>, _, Result<_>>(
            Vec::with_capacity(128),
            |mut acc, (path, v)| {
                for cls in v {
                    let text: String = cls.to_string();
                    let req = Request::new(text, "en-US".to_owned());
                    let resp = lt.check(req)?;
                    if let Some(software) = resp.software {
                        log::trace!("sw: {:?}", software);
                    }
                    if let Some(matches) = resp.matches {
                        for item in matches {
                            log::trace!("item.context: {:?}", item.context);
                            log::trace!("item.message: {:?}", item.message);
                            log::trace!("item.short_message: {:?}", item.short_message);
                            log::trace!("item.rule: {:?}", item.rule);
                            log::trace!("item.replacements: {:?}", item.rule);
                            // TODO convert response to offsets and errors with the matching literal
                            if let Some((literal, span)) = cls
                                .linear_coverage_to_span(item.offset as usize, item.length as usize)
                            {
                                acc.push(Suggestion {
                                    detector: Detector::LanguageTool,
                                    span: span,
                                    path: PathBuf::from(path),
                                    replacements: item
                                        .replacements
                                        .into_iter()
                                        .filter_map(|x| x.value)
                                        .collect(),
                                    literal: literal.into(),
                                });
                            } else {
                                warn!("Unable to map response to literal {} {}", item.offset , item.length)
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
