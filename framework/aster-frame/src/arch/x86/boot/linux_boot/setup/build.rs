use std::path::PathBuf;

fn main() {
    let source_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    println!(
        "cargo:rustc-link-arg-bins=--script={}",
        source_dir.join("linker.ld").display()
    )
}
