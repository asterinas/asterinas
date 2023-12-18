use std::{
    error::Error,
    io::Write,
    path::{Path, PathBuf},
};

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let source_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    build_linux_setup_header(&source_dir, &out_dir)?;
    Ok(())
}

fn build_linux_setup_header(
    source_dir: &Path,
    out_dir: &Path,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Build the setup header to ELF.
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
    cmd.arg("install").arg("jinux-frame-x86-boot-linux-setup");
    cmd.arg("--debug");
    cmd.arg("--locked");
    cmd.arg("--path").arg(setup_crate_dir.to_str().unwrap());
    cmd.arg("--target").arg(target_json.as_os_str());
    cmd.arg("-Zbuild-std=core,compiler_builtins");
    cmd.arg("-Zbuild-std-features=compiler-builtins-mem");
    // Specify the installation root.
    cmd.arg("--root").arg(out_dir.as_os_str());
    // Specify the build target directory to avoid cargo running
    // into a deadlock reading the workspace files.
    cmd.arg("--target-dir").arg(out_dir.as_os_str());
    cmd.env_remove("RUSTFLAGS");
    cmd.env_remove("CARGO_ENCODED_RUSTFLAGS");
    let output = cmd.output()?;
    if !output.status.success() {
        std::io::stdout().write_all(&output.stdout).unwrap();
        std::io::stderr().write_all(&output.stderr).unwrap();
        return Err(format!(
            "Failed to build linux x86 setup header:\n\tcommand `{:?}`\n\treturned {}",
            cmd, output.status
        )
        .into());
    }

    Ok(())
}
