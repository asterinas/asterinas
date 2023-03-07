use std::{env, error::Error};

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    limine_build_script()?;
    Ok(())
}

fn limine_build_script() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Have cargo rerun this script if the linker script or CARGO_PKG_ENV changes.
    println!("cargo:rerun-if-changed=boot/limine/conf/linker.ld");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_NAME");

    Ok(())
}
