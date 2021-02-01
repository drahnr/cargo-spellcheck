use std::path::PathBuf;
use std::env;
#[cfg(feature = "nlprules")]
mod nlprules {
    use super::*;
    use std::path::Path;

    use fs_err as fs;

    use std::io;
    use std::io::prelude::*;
    use flate2::read::GzDecoder;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum What {
        Tokenizer,
        Rules,
    }
    use std::fmt;

    impl fmt::Display for What {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(match self {
                Self::Tokenizer => "tokenizer",
                Self::Rules => "rules",
            })
        }
    }

    pub(crate) fn decompress(bytes: &[u8], dest: PathBuf) -> Result<(), io::Error> {
        let mut gz = GzDecoder::new(bytes);
        let mut buffer = Vec::with_capacity(bytes.len() >> 1);
        gz.read_to_end(&mut buffer)
            .expect("Decompression always works. qed");
        fs::write(dest, &buffer)?;
        Ok(())
    }

    pub(crate) fn get_resource(what: What, out: impl AsRef<Path>) -> Result<(), io::Error> {
        static NLPRULE_VERSION: &'static str = "0.3.0";
        static LANG_CODE: &'static str = "en";

        // TODO make this a local thing
        let response = reqwest::blocking::get(&format!(
            "https://github.com/bminixhofer/nlprule/releases/download/{}/{}_{}.bin.gz",
            NLPRULE_VERSION, LANG_CODE, what
        )).unwrap();
        let data = response.bytes().unwrap();

        let dest = out.as_ref().join(format!("{}.bin", what));
        decompress(&data[..], dest)?;

        Ok(())
    }
}
fn main() {
    let out = env::var("OUT_DIR").expect("OUT_DIR exists in env vars. qed");
    let out = PathBuf::from(out);

    #[cfg(feature = "nlprules")]
    nlprules::get_resource(nlprules::What::Tokenizer, &out).expect("Github download works. qed");

    #[cfg(feature = "nlprules")]
    nlprules::get_resource(nlprules::What::Rules, &out).expect("Github download works. qed");

    let _ = out;
}
