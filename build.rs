use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let out = env::var("OUT_DIR").expect("OUT_DIR exists in env vars. qed");
    let out = PathBuf::from(out);

    #[cfg(feature = "nlprules")]
    {
        nlprule_build::BinaryBuilder::new(Some(&["en"]), &out)
            .fallback_to_build_dir(true)
            .build()
            .validate();
    }

    let _ = out;
}
