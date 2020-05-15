//! The desired error output should look like this:
//!
//! ```raw
//! error[spellcheck]: Spelling
//! --> src/main.rs:138:16
//!     |
//! 138 | /// Thisf module is for easing the pain with printing text in the terminal.
//!     |     ^^^^^
//!     |     - The word "Thisf" is not in our dictionary. If you are sure this spelling is correct,
//!     |     - you can add it to your personal dictionary to prevent future alerts.
//! ```

use std::path::{Path, PathBuf};

use super::extractor::Documentation;

use anyhow::{anyhow, Result};

use languagetool::{LanguageTool, Request};

const HUNSPELL_AFF_DIR: &str = "/usr/share/myspell/";
const HUNSPELL_DIC_DIR: &str = "/usr/share/myspell/";

use hunspell::Hunspell;

use natural::tokenize;

#[derive(Default, Clone, Copy, Debug)]
pub struct Suggestion {
    dummy: u8,
}

pub(crate) fn check(docu: Documentation) -> Result<Vec<Suggestion>> {
    let grammar: bool = false;
    let spelling: bool = true;
    let mut corrections = Vec::<Suggestion>::with_capacity(128);


	let literal_to_string = |literal: &proc_macro2::Literal| -> String { format!("{}", literal) };
	let literals_to_string = |literals: &[proc_macro2::Literal]| -> String {
		literals
		.into_iter()
		.map(literal_to_string)
		.collect::<Vec<String>>()
		.join("\n") };
    if grammar {
        // TODO make configurable
        let url = "https://127.0.0.1:1337";
        let lt = LanguageTool::new(url)?;
        let mut suggestions =
            docu.iter()
                .try_fold::<Vec<Suggestion>,_,Result<_>>(Vec::with_capacity(128), |mut acc, (path, literals)| {
                    let text: String = literals_to_string(literals.as_slice());
                    // let text = text.as_str();
                    let req = Request::new(text, "en-US".to_owned());
                    let resp = lt.check(req)?;
                    let _ = dbg!(resp);
                    // TODO convert response to offsets and errors
                    acc.push(Suggestion::default());
                    Ok(acc)
                })?;

        corrections.append(&mut suggestions);
    }

    if spelling {
        // TODO make configurable
        let lang = "en_US";
        let mut aff_file = PathBuf::from(HUNSPELL_AFF_DIR).join(lang);
        aff_file.set_extension("aff");
        let mut dic_file = PathBuf::from(HUNSPELL_DIC_DIR).join(lang);
        dic_file.set_extension("dic");

        let hunspell = Hunspell::new(aff_file.to_str().unwrap(), dic_file.to_str().unwrap());
        let mut suggestions = docu
            .iter()
            .fold(Vec::with_capacity(128), |mut acc, (path, literals)| {
				let text: String = literals_to_string(literals.as_slice());
                let text = text.as_str();
                let words = tokenize::tokenize(text);
                for word in words {
                    if !hunspell.check(word) {
                        let _ = dbg!(hunspell.suggest(word));
                        // FIXME TODO convert results
                    }
                }
                Vec::new()
            });

        corrections.append(&mut suggestions);
    }
    // TODO remove overlapping
    Ok(dbg!(corrections))
}
