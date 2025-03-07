// SPDX-License-Identifier: MPL-2.0

use std::{path::PathBuf, process::Command};

fn main() {
    let source_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    let target_arch = std::env::var("TARGET").unwrap();
    let (lds, def) = match target_arch.as_str() {
        "x86_64-unknown-none" => (
            source_dir.join("src/x86/linker.ld"),
            "-DCFG_TARGET_ARCH_X86_64=1",
        ),
        "x86_64-i386_pm-none" => (
            source_dir.join("src/x86/linker.ld"),
            "-DCFG_TARGET_ARCH_X86_64=0",
        ),
        other => panic!("unsupported target: {}", other),
    };

    let out_lds = out_dir.join("linker.rs");
    let status = Command::new("cpp")
        .arg("-o")
        .arg(&out_lds)
        .arg(def)
        .arg(&lds)
        .status()
        .expect("failed to run the preprocessor");
    assert!(status.success(), "the preprocessor exits with failure");

    println!("cargo:rerun-if-changed={}", lds.display());
    println!("cargo:rustc-link-arg=-T{}", out_lds.display());
}
