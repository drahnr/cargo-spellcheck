use super::*;

const HUNSPELL_AFF_DIR: &str = "/usr/share/myspell/";
const HUNSPELL_DIC_DIR: &str = "/usr/share/myspell/";

use hunspell_rs::Hunspell;

pub struct HunspellChecker;

impl Checker for HunspellChecker {
    fn check<'a, 's>(docu: &'a Documentation) -> Result<Vec<Suggestion<'s>>>
    where
        'a: 's,
    {
        // TODO make configurable
        let lang = "en_US";
        let mut aff_file = PathBuf::from(HUNSPELL_AFF_DIR).join(lang);
        aff_file.set_extension("aff");
        let mut dic_file = PathBuf::from(HUNSPELL_DIC_DIR).join(lang);
        dic_file.set_extension("dic");

        let hunspell = Hunspell::new(
            aff_file.to_str().expect(".aff file must exist"),
            dic_file.to_str().expect(".dic file must exist"),
        );
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
                            // FIXME translate the rspan back to
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
