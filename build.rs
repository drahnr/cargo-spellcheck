use std::env;
use std::io::BufReader;
use std::path::PathBuf;
use xz2::bufread::{XzDecoder, XzEncoder};

fn extract_version<'a>(manifest: &'a cargo_toml::Manifest, pkg_name: &str) -> Option<&'a str> {
    let (_, dependency) = manifest
        .dependencies
        .get_key_value(pkg_name)?;
    let version = dependency
        .detail()
            .map(|x| x.version.as_ref().map(|x| x.as_str())).flatten()?;
    let version = match version {
        x if x.starts_with("=") => &version[1..],
        x if x.starts_with("<=") || x.starts_with(">=") => &version[2..],
        x if x.starts_with("*") => panic!("Don't be silly."),
        _ => version,
    };
    Some(version)
}

fn main() -> std::result::Result<(), Box<(dyn std::error::Error + 'static)>> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");

    let out = env::var("OUT_DIR").expect("OUT_DIR exists in env vars. qed");
    let out = PathBuf::from(out);

    const MISSING: &str = "NOT COMPILED IN";
    if !cfg!(feature = "hunspell") {
        println!("cargo:rustc-env=CHECKER_NLPRULE_VERSION={}", MISSING);
    }
    if !cfg!(feature = "nlprules") {
        println!("cargo:rustc-env=CHECKER_HUNSPELL_VERSION={}", MISSING);
    }

    // extract the version from the manifest.
    // only accept defined versions, no git dependencies or whatever
    let manifest = std::path::PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST exists in env vars. qed")).join("Cargo.toml");
    let manifest = cargo_toml::Manifest::from_path(manifest)?;
    #[cfg(feature = "hunspell")]
    {
        let version = extract_version(&manifest, "hunspell-rs").expect("Hunspell must be present. qed");
        println!("cargo:rustc-env=CHECKER_HUNSPELL_VERSION={}", version);
    }
    #[cfg(feature = "nlprules")]
    {
        let nlprule_version = extract_version(&manifest, "nlprule").expect("Hunspell must be present. qed");
        println!("cargo:rustc-env=CHECKER_NLPRULE_VERSION={}", nlprule_version);

        const COMPRESSION_EXTENSION: &str = "xz";
        const ARTIFACTS_DIR: &str = "nlprule-data";

        let cwd = env::current_dir().expect("Current dir exists. qed");
        let cache_dir = cwd.join(ARTIFACTS_DIR).join(nlprule_version).join("en");
        std::fs::create_dir_all(&cache_dir)?;
        println!("cargo:rerun-if-changed={}/en_rules.bin.{}", cache_dir.display(), COMPRESSION_EXTENSION);
        println!("cargo:rerun-if-changed={}/en_tokenizer.bin.{}", cache_dir.display(), COMPRESSION_EXTENSION);

        println!("cargo:rerun-if-changed={}/en_rules.bin", out.display());
        println!("cargo:rerun-if-changed={}/en_tokenizer.bin", out.display());

        let builder = nlprule_build::BinaryBuilder::new(&["en"], &out);
        builder
            .version(nlprule_version)
            .out_dir(out)
            .fallback_to_build_dir(false)
            .cache_dir(Some(cache_dir))
            .transform(
                |source, mut sink| {
                    let mut encoder = XzEncoder::new(BufReader::new(source), 9);
                    std::io::copy(&mut encoder, &mut sink)?;
                    Ok(())
                },
                |mut path: PathBuf| -> Result<PathBuf, Box<(dyn std::error::Error + Send + Sync + 'static)>> {
                    let mut ext = path.extension().map(|s| {
                        s.to_os_string()
                            .into_string()
                            .expect("Extension conversion from OSString to regular string works. qed") })
                            .unwrap_or_else(|| String::with_capacity(4));

                    ext.push('.');
                    ext.push_str(COMPRESSION_EXTENSION);
                    path.set_extension(ext);
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
                    let ext = path.extension()
                        .map(|s| { s.to_os_string().into_string().expect("Extension conversion from OSString to regular string works. qed") })
                        .unwrap_or_else(|| String::with_capacity(4));

                    assert!(&ext.ends_with(COMPRESSION_EXTENSION));
                    let k = ext.len().saturating_sub(COMPRESSION_EXTENSION.len() + 1);
                    path.set_extension(&ext[..k]);
                    path
                }
            )?
            .validate()?;
    }

    let _ = out;
    Ok(())
}
