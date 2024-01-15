// Licensed under the Apache License, Version 2.0 or the MIT License.
// Copyright (C) 2023-2024 Ant Group.

// This implementation is from rust clippy. We modified the code.

fn main() {
    // Forward the profile to the main compilation
    println!(
        "cargo:rustc-env=PROFILE={}",
        std::env::var("PROFILE").unwrap()
    );
    // Don't rebuild even if nothing changed
    println!("cargo:rerun-if-changed=build.rs");
    rustc_tools_util::setup_version_info!();
}
