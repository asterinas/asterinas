use std::{error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let linker_script_path = get_source_dir()
        .join("framework")
        .join("jinux-frame")
        .join("src")
        .join("arch")
        .join("x86")
        .join("boot")
        .join("linker.ld");
    println!(
        "cargo:rerun-if-changed={}",
        linker_script_path.to_str().unwrap()
    );
    println!(
        "cargo:rustc-link-arg=-T{}",
        linker_script_path.to_str().unwrap()
    );
    println!("cargo:rerun-if-env-changed=CARGO_PKG_NAME");
    Ok(())
}

fn get_source_dir() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    PathBuf::from(manifest_dir)
}
