// SPDX-License-Identifier: MPL-2.0

use std::process::Command;

use crate::util::{get_kernel_crate, new_command_checked_exists};

/// Since the pre-compiled `libcore` distribution for bare-metal targets
/// defaults to the `abort` panic strategy, `cargo osdk test` requires a
/// recompile via `-Zbuild-std` to enable the `unwind` strategy.
/// This is necessary for the kernel to catch `#[should_panic]` tests;
/// otherwise, `gimli` will be unable to perform stack backtraces, preventing
/// the `unwinding` crate from intercepting panics.
pub const OSDK_TEST_CARGO_ARGS: &[&str] = &["-Zbuild-std=core,alloc"];

pub const DEFAULT_TARGET_RELPATH: &str = "osdk";

pub fn cargo() -> Command {
    new_command_checked_exists("cargo")
}

pub fn profile_name_adapter(profile: &str) -> &str {
    match profile {
        "dev" => "debug",
        _ => profile,
    }
}

pub fn bin_file_name() -> String {
    get_kernel_crate().name + "-osdk-bin"
}

pub(crate) fn is_tdx_enabled() -> bool {
    std::env::var("INTEL_TDX").is_ok_and(|s| s == "1")
}
