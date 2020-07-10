//! Wrap
//!
//! Re-wrap doc comments for prettier styling in code

use anyhow::Result;

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
pub struct Wrap {}

impl Wrap {
    pub fn rewrap(doc: Documentation, config: Option<WrapConfig>) -> Result<()> {
        let config = config.unwrap_or_default();
        // loop through files
        for (origin, chunks) in doc.iter() {
            // loop through chunks a.k.a one conn
            for chunk in chunks {
                let new_chunks: Vec<String> = chunk.as_str().split("\n\n").collect::<Vec<&str>>().iter().map(|s| {
                    let comment = s.replace("\n", "");
                    textwrap::fill(&comment, config.max_line_length)
                }).collect();
                println!("{:?}", new_chunks.first());
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

        const TEST: &str = include_str!("../../demo/src/nested/just_very_long.rs");

        let stream =
            syn::parse_str::<proc_macro2::TokenStream>(TEST).expect("Must parse just fine");

        let d = Documentation::from((
            ContentOrigin::RustSourceFile(PathBuf::from("dummy/dummy.rs")),
            stream,
        ));

        Wrap::rewrap(d, None).expect("failed");

        assert!(false);
    }
}