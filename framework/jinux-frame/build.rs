use std::{error::Error, io::Write, path::PathBuf};

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    build_linux_setup_header()?;
    Ok(())
}

fn get_source_dir() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    PathBuf::from(manifest_dir)
}

fn get_header_out_dir() -> PathBuf {
    PathBuf::from(std::env::var("OUT_DIR").unwrap())
}

fn build_linux_setup_header() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Compile the header to raw binary.
    let linux_boot_header_asm_path = get_source_dir()
        .join("src")
        .join("arch")
        .join("x86")
        .join("boot")
        .join("linux_boot")
        .join("header.S");
    println!(
        "cargo:rerun-if-changed={}",
        linux_boot_header_asm_path.to_str().unwrap()
    );
    let linux_boot_header_elf_path = get_header_out_dir().join("linux_header.o");
    let gas = std::env::var("AS").unwrap();
    let mut cmd = std::process::Command::new(gas);
    cmd.arg(linux_boot_header_asm_path);
    cmd.arg("-o")
        .arg(linux_boot_header_elf_path.to_str().unwrap());
    let output = cmd.output()?;
    if !output.status.success() {
        std::io::stdout().write_all(&output.stdout).unwrap();
        std::io::stderr().write_all(&output.stderr).unwrap();
        panic!(
            "Failed to compile linux boot header:\n\tcommand `{:?}`\n\treturned {}",
            cmd, output.status
        );
    }
    // Strip the elf header to get the raw header.
    let linux_boot_header_bin_path = get_header_out_dir().join("linux_header.bin");
    let objcopy = std::env::var("OBJCOPY").unwrap();
    let mut cmd = std::process::Command::new(objcopy);
    cmd.arg("-O").arg("binary");
    cmd.arg("-j").arg(".boot_compatibility_bin");
    cmd.arg(linux_boot_header_elf_path.to_str().unwrap());
    cmd.arg(linux_boot_header_bin_path.to_str().unwrap());
    let output = cmd.output()?;
    if !output.status.success() {
        std::io::stdout().write_all(&output.stdout).unwrap();
        std::io::stderr().write_all(&output.stderr).unwrap();
        panic!(
            "Failed to strip linux boot header:\n\tcommand `{:?}`\n\treturned {}",
            cmd, output.status
        );
    }
    Ok(())
}
