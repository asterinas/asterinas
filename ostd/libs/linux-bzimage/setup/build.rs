// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

fn main() {
    let source_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let target_arch = std::env::var("TARGET").unwrap();
    let linker_script = if target_arch == "x86_64-unknown-none" {
        source_dir.join("src/x86/amd64_efi/linker.ld")
    } else if target_arch == "x86_64-i386_pm-none" {
        source_dir.join("src/x86/legacy_i386/linker.ld")
    } else {
        panic!("Unsupported target_arch: {}", target_arch);
    };
    println!("cargo:rerun-if-changed={}", linker_script.display());
    println!("cargo:rustc-link-arg=-T{}", linker_script.display());
}
