// SPDX-License-Identifier: MPL-2.0

use std::process::Command;

use crate::util::{get_kernel_crate, new_command_checked_exists};

pub const COMMON_CARGO_ARGS: &[&str] = &[
    "-Zbuild-std=core,alloc,compiler_builtins",
    "-Zbuild-std-features=compiler-builtins-mem",
];

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
