use fs_err as fs;
use std::env;
use std::io::{self, BufReader};
use std::path::PathBuf;
use xz2::bufread::{XzDecoder, XzEncoder};

fn main() -> std::result::Result<(), Box<(dyn std::error::Error + 'static)>> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");

    let out = env::var("OUT_DIR").expect("OUT_DIR exists in env vars. qed");
    let out = PathBuf::from(out);

    #[cfg(feature = "nlprules")]
    {
        println!("cargo:rerun-if-changed=nlprule-data/en_rules.bin.xz");
        println!("cargo:rerun-if-changed=nlprule-data/en_tokenizer.bin.xz");

        println!("cargo:rerun-if-changed={}/en_rules.bin", out.display());
        println!("cargo:rerun-if-changed={}/en_tokenizer.bin", out.display());

        let cwd = env::current_dir().expect("Current dir must exist. qed");

        const COMPRESSION_EXTENSION: &str = "xz";
        const VERSION: &str = "0.4.6";
        const ARTIFACTS_DIR: &str = "nlprule-data";

        let artifacts = cwd.join(ARTIFACTS_DIR);
        let cache_dir = if cfg!(feature = "artifacts") {
            // update the artifacts in git
            Some(artifacts)
        } else {
            // since cargo publish is not happy about any files outside
            // of $OUT being touched with the build.rs
            let artifacts = artifacts.join(VERSION).join("en");
            let tmp_dest = out.join(ARTIFACTS_DIR);
            let tmp_dist_sub = tmp_dest.join(VERSION).join("en");
            fs::create_dir_all(&tmp_dist_sub)?;

            let cpy = |from: &PathBuf, to: &PathBuf, what: &str| -> io::Result<()> {
                fs::copy(dbg!(from.join(what)), dbg!(to.join(what)))?;
                Ok(())
            };
            cpy(&artifacts, &tmp_dist_sub, "en_rules.bin.xz")?;
            cpy(&artifacts, &tmp_dist_sub, "en_tokenizer.bin.xz")?;
            Some(tmp_dest)
        };

        nlprule_build::BinaryBuilder::new(&["en"], &out)
            .version(VERSION)
            .fallback_to_build_dir(false)
            .cache_dir(cache_dir)
            .transform(
                &|source, mut sink| {
                    eprintln!("Calling transform data");
                    let mut encoder = XzEncoder::new(BufReader::new(source), 9);
                    std::io::copy(&mut encoder, &mut sink)?;
                    Ok(())
                },
                &|mut path: PathBuf| -> Result<PathBuf, Box<(dyn std::error::Error + Send + Sync + 'static)>> {
                    let mut ext = path.extension().map(|s| {
                        s.to_os_string()
                            .into_string()
                            .expect("Extension conversion from OSString to regular string works. qed") })
                            .unwrap_or_else(|| String::with_capacity(4));

                    ext.push_str(".");
                    ext.push_str(COMPRESSION_EXTENSION);
                    path.set_extension(ext);
                    eprintln!("transform: {}", path.display());
                    Ok(path)
                })
            .build()?
            .postprocess(
                |source, mut sink| {
                    let mut decoder = XzDecoder::new(BufReader::new(source));
                    std::io::copy(&mut decoder, &mut sink)?;
                    Ok(())
                },
                |mut path: PathBuf| -> PathBuf {
                    eprintln!("Calling postprocess data");
                    let ext = path.extension()
                        .map(|s| { s.to_os_string().into_string().expect("Extension conversion from OSString to regular string works. qed") })
                        .unwrap_or_else(|| String::with_capacity(4));

                    assert!(dbg!(&ext).ends_with(COMPRESSION_EXTENSION));
                    let k = ext.len().saturating_sub(COMPRESSION_EXTENSION.len() + 1);
                    path.set_extension(&ext[..k]);
                    eprintln!("postprocess: {}", path.display());
                    path
                }
            )?
            .validate()?;
    }

    let _ = out;
    Ok(())
}
