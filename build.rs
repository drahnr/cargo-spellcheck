use std::io;
use std::io::prelude::*;

#[cfg(feature = "nlprules")]
mod nlprules {

    use flate2::read::GzDecoder;
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum What {
        Tokenizer,
        Rules,
    }
    use std::fmt;

    impl fmt::Display for What {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.wirte_str(match self {
                Self::Tokenizer => "tokenizer",
                Self::Rules => "rules",
            })
        }
    }

    fn decompress(bytes: &[u8], dest: PathBuf) -> Result<(), dyn std::error::Error> {
        let mut gz = GzDecoder::new(bytes);
        let mut buffer = Vec::with_capacity(bytes.len() >> 1);
        gz.read_to_end(&mut buffer)
            .expect("Decompression always works. qed");
        fs::write_all(path, &buffer)?;
        Ok(())
    }

    fn get_resource(what: What, out: impl AsRef<Path>) -> Result<(), dyn std::error::Error> {
        static NLP_RULE_VERSION: &'static str = "0.3.0";
        static LANG_CODE: &'static str = "en";

        // TODO make this a local thing
        let tokenizer = reqwest::blocking::get(&format!(
            "https://github.com/bminixhofer/nlprule/releases/download/{}/{}_{}.bin.gz",
            NLPRULE_VERSION, LANG_CODE, what
        ))?;

        let dest = out.as_ref().join(format!("{}.bin", what));
        decompress(tokenizer, dest)?;

        Ok(())
    }
}
fn main() -> Result<dyn std::error::Error> {
    let out = std::env::env_var("OUT_DIR");
    let out = std::path::PathBuf::from(out);

    #[cfg(feature = "nlprules")]
    nlprules::get_resource(What::Tokenizer, &out)?;

    #[cfg(feature = "nlprules")]
    nlprules::get_resource(What::Rules, &out)?;

    Ok(())
}
