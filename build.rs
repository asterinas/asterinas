use std::{error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let target = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let linker_script_path = if target == "x86_64" {
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("framework")
            .join("jinux-frame")
            .join("src")
            .join("arch")
            .join("x86")
            .join("linker.ld")
    } else {
        panic!("Unsupported target arch: {}", target);
    };
    println!("cargo:rerun-if-changed={}", linker_script_path.display());
    println!("cargo:rustc-link-arg=-T{}", linker_script_path.display());
    Ok(())
}
