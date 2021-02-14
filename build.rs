use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let out = env::var("OUT_DIR").expect("OUT_DIR exists in env vars. qed");
    let out = PathBuf::from(out);

    #[cfg(feature = "nlprules")]
    {
        let cwd = env::current_dir().expect("Current dir must exist. qed");
        let cache_dir = cwd.join("nlprule-data");

        nlprule_build::BinaryBuilder::new(None, &out)
            .fallback_to_build_dir(true)
            .cache_dir(None)
            .build()
            .validate();
    }

    let _ = out;
}
