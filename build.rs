use std::env;
use std::path::PathBuf;
#[cfg(feature = "nlprules")]
mod nlprules {
    use std::path::{Path, PathBuf};

    use fs_err as fs;

    use flate2::read::GzDecoder;
    use std::env;
    use std::io;
    use std::io::prelude::*;

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

    pub(crate) fn decompress(bytes: &[u8], dest: &Path) -> Result<(), io::Error> {
        let mut gz = GzDecoder::new(bytes);
        let mut buffer = Vec::with_capacity(bytes.len() >> 1);
        gz.read_to_end(&mut buffer)
            .expect("Decompression always works. qed");
        let mut f = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(dest)?;
        f.write_all(&buffer)?;
        Ok(())
    }

    pub(crate) fn get_resource(what: What, out: impl AsRef<Path>) -> Result<PathBuf, io::Error> {
        static NLPRULE_VERSION: &'static str = "0.3.0";
        static LANG_CODE: &'static str = "en";

        let file_name = format!("{}.bin", what);
        let dest = out.as_ref().join(&file_name);

        println!("cargo:rerun-if-changed={}", dest.display());

        if dest.is_file() {
            return Ok(dest);
        }

        let manifest_dir =
            PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo manifest dir ist set. qed"));
        let alt = manifest_dir
            .join("nlprule-data")
            .join(format!("{}_{}", LANG_CODE, &file_name));
        if alt.is_file() {
            fs::copy(alt, &dest)?;
            return Ok(dest);
        }

        let url = &format!(
            "https://github.com/bminixhofer/nlprule/releases/download/{}/{}_{}.bin.gz",
            NLPRULE_VERSION, LANG_CODE, what
        );
        let data = reqwest::blocking::get(url)
            .ok()
            .and_then(|response| {
                if response.status().as_u16() != 200_u16 {
                    eprintln!("http status: {:?}", response.status());
                    return None;
                }
                Some(
                    response
                        .bytes()
                        .expect("HTTP response contains payload. qed")
                        .to_vec(),
                )
            })
            .unwrap_or_else(|| {
                let dest = manifest_dir
                .join(file_name)
                // .join("nlprule-data")
                ;
                let mut f = fs::OpenOptions::new().read(true).open(dest).unwrap();
                let mut buf = Vec::with_capacity(10 << 10);
                f.read_to_end(&mut buf).unwrap();
                buf
            });

        decompress(&data[..], &dest).unwrap();

        Ok(dest)
    }
}
fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let out = env::var("OUT_DIR").expect("OUT_DIR exists in env vars. qed");
    let out = PathBuf::from(out);

    #[cfg(feature = "nlprules")]
    {
        let loco = nlprules::get_resource(nlprules::What::Tokenizer, &out)
            .expect("Github download works. qed");
        let _ = nlprule::Tokenizer::new(&loco)
            .expect("build.rs pulls valid tokenizer description. qed");

        let loco = nlprules::get_resource(nlprules::What::Rules, &out)
            .expect("Github download works. qed");
        let _ = nlprule::Rules::new(&loco).expect("build.rs pulls valid rules description. qed");
    }

    let _ = out;
}
