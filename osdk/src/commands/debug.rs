// SPDX-License-Identifier: MPL-2.0

use crate::commands::util::{bin_file_name, profile_adapter};
use crate::config_manager::DebugConfig;

use crate::util::get_target_directory;
use std::process::Command;

pub fn execute_debug_command(config: &DebugConfig) {
    let DebugConfig {
        manifest,
        cargo_args,
    } = config;

    let profile = profile_adapter(&cargo_args.profile);
    let file_path = get_target_directory()
        .join("x86_64-unknown-none")
        .join(profile)
        .join(bin_file_name());
    println!("Debugging {:?}", file_path);

    let mut gdb = Command::new("gdb");
    gdb.args([
        "-ex",
        "target remote :1234",
        "-ex",
        format!("file {}", file_path.display()).as_str(),
    ]);
    gdb.status().unwrap();
}
