//! Wrap
//!
//! Re-wrap doc comments for prettier styling in code

use anyhow::Result;
use log::info;
use indexmap::IndexMap;
use std::ops::Range;
use crate::Span;

use crate::Documentation;

/// Parameters for wrapping doc comments
#[derive(Debug)]
pub struct WrapConfig {
    /// Hard limit for absolute length of lines
    max_line_length: usize,
}

impl Default for WrapConfig {
    fn default() -> Self {
        WrapConfig { max_line_length: 70 }
    }
}

#[derive(Debug)]
pub struct Wrap {
    doc: Documentation,
    config: WrapConfig,
}

impl Wrap {
    pub fn new(doc: Documentation, config: Option<WrapConfig>) -> Self {
        Wrap { doc, config: config.unwrap_or_default() }
    }

    pub fn rewrap(self) -> Result<()> {
        // loop through files
        for (origin, chunks) in self.doc.iter() {
            for chunk in chunks {
                // check length
                let oob = chunk.iter().filter(|(r, s)| {
                    s.end.column > self.config.max_line_length
                }).map(|(r, s)| {
                    for range in crate::checker::tokenize(chunk.as_str()) {
                        // get all words with span > max_line_length
                    }

                });

                log::warn!("oob {:?}", oob);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documentation::*;
    use std::path::PathBuf;

    #[test]
    fn rewrap() {
        let _ = env_logger::builder()
            .filter(None, log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        const TEST: &str = include_str!("../../demo/src/lib.rs");

        let stream =
            syn::parse_str::<proc_macro2::TokenStream>(TEST).expect("Must parse just fine");

        let d = Documentation::from((
            ContentOrigin::RustSourceFile(PathBuf::from("dummy/dummy.rs")),
            stream,
        ));

        let wrapper = Wrap::new(d, None);
        wrapper.rewrap().expect("failed");

        assert!(false);
    }
}