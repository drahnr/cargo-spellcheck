use std::env;
use std::path::PathBuf;
use std::io::BufReader;

use xz2::bufread::{XzEncoder, XzDecoder};


fn main() -> std::result::Result<(), Box<(dyn std::error::Error + 'static)>> {
    println!("cargo:rerun-if-changed=build.rs");

    let out = env::var("OUT_DIR").expect("OUT_DIR exists in env vars. qed");
    let out = PathBuf::from(out);

    #[cfg(feature = "nlprules")]
    {
        let cwd = env::current_dir().expect("Current dir must exist. qed");
        let cache_dir = cwd.join("nlprule-data");

        const COMPRESSION_EXTENSION: &str = "xz";

        nlprule_build::BinaryBuilder::new(&["en"], &out)
            .fallback_to_build_dir(true)
            .cache_dir(Some(cache_dir))
            .transform(
                &|source, mut sink| {
                    eprintln!("Calling transform data");
                    let mut encoder = XzEncoder::new(BufReader::new(source), 8);
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
            // .validate()? requires https://github.com/bminixhofer/nlprule/pull/39
            ;
    }

    let _ = out;
    Ok(())
}
