use super::*;

use log::trace;

use hunspell_rs::Hunspell;

pub struct HunspellChecker;

impl Checker for HunspellChecker {
    type Config = crate::config::HunspellConfig;
    fn check<'a, 's>(docu: &'a Documentation, config: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's,
    {
        // let hunspell = lazy_static::lazy_static!{
        //     static ref HUNSPELL_CTX: Result<Hunspell> = {

        //     }
        // };

        let search_dirs = config.search_dirs();
        if search_dirs.is_empty() {
            return Err(anyhow::anyhow!(
                "Need some search dirs defined for Hunspell"
            ));
        }

        let search_path_one: &PathBuf = search_dirs.first().unwrap();
        let aff = search_path_one.join(format!("{}.aff", config.lang()));
        let dic = search_path_one.join(format!("{}.dic", config.lang()));
        let mut hunspell = Hunspell::new(aff.to_str().unwrap(), dic.to_str().unwrap());
        for search_dir in search_dirs.iter().skip(1) {
            if !search_dir.is_dir() {
                return Err(anyhow::anyhow!(
                    "Dictionary search path {} is not a directory",
                    search_dir.display()
                ));
            }
            let dic = search_dir.join(format!("{}.dic", config.lang()));
            if !dic.is_file() {
                return Err(anyhow::anyhow!(
                    "Extra dictionary {} is not a file",
                    dic.display()
                ));
            }
            if let Some(dic) = dic.to_str() {
                hunspell.add_dictionary(dic);
            } else {
                return Err(anyhow::anyhow!(
                    "Failed to convert one of the base dictionaries to a str"
                ));
            }
        }
        for extra_dic in config.extra_dictonaries().iter() {
            if !extra_dic.is_file() {
                return Err(anyhow::anyhow!(
                    "Extra dictionary {} is not a file",
                    dic.display()
                ));
            }
            if let Some(extra_dic) = extra_dic.to_str() {
                hunspell.add_dictionary(extra_dic);
            } else {
                return Err(anyhow::anyhow!(
                    "Failed to convert one of the extra dictionaries to a str"
                ));
            }
        }

        let suggestions = docu.iter().try_fold::<SuggestionSet, _, Result<_>>(
            SuggestionSet::new(),
            |mut acc, (path, literal_sets)| {
                for literal_set in literal_sets {
                    let plain = literal_set.erase_markdown();
                    trace!("{:?}", &plain);
                    let txt = dbg!(plain.as_str());
                    for range in dbg!(tokenize(txt)) {
                        let word = &txt[range.clone()];
                        if !hunspell.check(word) {
                            trace!("No match for word (plain range: {:?}): {}", &range, word);
                            // get rid of single character suggestions
                            let replacements = hunspell
                                .suggest(word)
                                .into_iter()
                                .filter(|x| x.len() > 1) // single char suggestions tend to be useless
                                .collect::<Vec<_>>();

                            for (literal, span) in plain.linear_range_to_spans(range.clone()) {
                                acc.add(
                                    path.to_owned(),
                                    Suggestion {
                                        detector: Detector::Hunspell,
                                        span,
                                        path: PathBuf::from(path),
                                        replacements: replacements.clone(),
                                        literal: literal.into(),
                                        description: Some(
                                            "Possible spelling mistake found.".to_owned(),
                                        ),
                                    },
                                )
                            }
                        } else {
                            trace!("Found a match for word (plain range: {:?}): {}", &range, word);
                        }
                    }
                }
                Ok(acc)
            },
        )?;

        // TODO sort spans by file and line + column
        Ok(suggestions)
    }
}
