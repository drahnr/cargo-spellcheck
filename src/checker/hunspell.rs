use super::*;

use hunspell_rs::Hunspell;

pub struct HunspellChecker;

impl Checker for HunspellChecker {
    type Config = crate::config::HunspellConfig;
    fn check<'a, 's>(docu: &'a Documentation, config: &Self::Config) -> Result<Vec<Suggestion<'s>>>
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

        let suggestions = docu
            .iter()
            .fold(Vec::with_capacity(128), |mut acc, (path, literals)| {
                // FIXME literals should be passed directly to tokenize to allow
                // for correct span calculation
                for (words_with_spans, literal) in tokenize_literals(literals) {
                    for (word, rspan) in words_with_spans {
                        let word = word.as_str();
                        if !hunspell.check(word) {
                            // get rid of single character suggestions
                            let replacements = hunspell
                                .suggest(word)
                                .into_iter()
                                .filter(|x| x.len() != 1)
                                .collect::<Vec<_>>();

                            acc.push(Suggestion {
                                detector: Detector::Hunspell,
                                span: rspan,
                                path: PathBuf::from(path),
                                replacements,
                                literal,
                                description: Some("Possible spelling mistake found.".to_owned()),
                            })
                        }
                    }
                }
                acc
            });

        // TODO sort spans by file and line + column
        Ok(suggestions)
    }
}
