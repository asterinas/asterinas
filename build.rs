use std::error::Error;

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let linker_script_path = "build/grub/conf/linker.ld";
    println!("cargo:rerun-if-changed={}", linker_script_path);
    println!("cargo:rustc-link-arg=-T{}", linker_script_path);
    println!("cargo:rerun-if-env-changed=CARGO_PKG_NAME");

    Ok(())
}
