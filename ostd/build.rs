// SPDX-License-Identifier: MPL-2.0

use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    // Only run this script for loongarch64 target architecture
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if target_arch != "loongarch64" {
        // Silently skip DTB generation for non-LoongArch targets
        return;
    }

    println!("cargo:rerun-if-changed=build.rs"); // Ensure rerun when build.rs changes
    println!("cargo:info=Generating DTB for LoongArch target");

    // The output directory: target/**/build/<crate-name>/out
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let dtb_path = out_dir.join("loongarch_virt.dtb");

    // Remove the existing file if it already exists
    let _ = fs::remove_file(&dtb_path);

    // Generate the device tree binary using QEMU
    // FIXME: Synchronize with commands in OSDK.toml
    let status = Command::new("qemu-system-loongarch64")
        .args([
            "-m",
            "8G",
            "-smp",
            "1",
            "--no-reboot",
            "-nographic",
            "-display",
            "none",
            "-rtc",
            "base=utc",
            "-machine",
            &format!("virt,dumpdtb={}", dtb_path.display()),
        ])
        .status()
        .expect("Failed to execute qemu-system-loongarch64");

    if !status.success() {
        panic!("QEMU failed to generate DTB: {:?}", status);
    }

    println!(
        "cargo:info=DTB successfully generated at {}",
        dtb_path.display()
    );
}
