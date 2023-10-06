use std::{error::Error, io::Write, path::PathBuf};

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    build_linux_setup_header()?;
    Ok(())
}

fn build_linux_setup_header() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Build the setup header to raw binary.
    let source_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let setup_crate_dir = source_dir
        .join("src")
        .join("arch")
        .join("x86")
        .join("boot")
        .join("linux_boot")
        .join("setup");
    let target_json = setup_crate_dir.join("x86_64-i386_protected_mode.json");

    println!(
        "cargo:rerun-if-changed={}",
        setup_crate_dir.to_str().unwrap()
    );

    let cargo = std::env::var("CARGO").unwrap();
    let mut cmd = std::process::Command::new(cargo);
    cmd.arg("install").arg("jinux-frame-x86-boot-setup");
    cmd.arg("--locked");
    cmd.arg("--path").arg(setup_crate_dir.to_str().unwrap());
    cmd.arg("--target").arg(target_json.as_os_str());
    cmd.arg("-Zbuild-std=core,compiler_builtins");
    cmd.arg("-Zbuild-std-features=compiler-builtins-mem");
    cmd.arg("--root").arg(out_dir.as_os_str());
    cmd.env_remove("RUSTFLAGS");
    cmd.env_remove("CARGO_ENCODED_RUSTFLAGS");
    let output = cmd.output()?;
    if !output.status.success() {
        std::io::stdout().write_all(&output.stdout).unwrap();
        std::io::stderr().write_all(&output.stderr).unwrap();
        return Err(format!(
            "Failed to build linux x86 setup header::\n\tcommand `{:?}`\n\treturned {}",
            cmd, output.status
        )
        .into());
    }

    // Strip the elf header to get the raw header.
    let elf_path = out_dir.join("bin").join("jinux-frame-x86-boot-setup");
    let bin_path = out_dir.join("bin").join("jinux-frame-x86-boot-setup.bin");

    let objcopy = std::env::var("OBJCOPY").unwrap();
    let mut cmd = std::process::Command::new(objcopy);
    cmd.arg("-O").arg("binary");
    cmd.arg("-j").arg(".boot_real_mode");
    cmd.arg(elf_path.to_str().unwrap());
    cmd.arg(bin_path.to_str().unwrap());
    let output = cmd.output()?;
    if !output.status.success() {
        std::io::stdout().write_all(&output.stdout).unwrap();
        std::io::stderr().write_all(&output.stderr).unwrap();
        return Err(format!(
            "Failed to strip linux boot header:\n\tcommand `{:?}`\n\treturned {}",
            cmd, output.status
        )
        .into());
    }
    Ok(())
}
