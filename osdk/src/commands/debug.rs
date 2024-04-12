// SPDX-License-Identifier: MPL-2.0

use crate::commands::util::bin_file_name;

use crate::{cli::DebugArgs, util::get_target_directory};
use std::process::Command;

pub fn execute_debug_command(profile: &String, args: &DebugArgs) {
    let remote = &args.remote;

    let file_path = get_target_directory()
        .join("x86_64-unknown-none")
        .join(profile)
        .join(bin_file_name());
    println!("Debugging {}", file_path.display());

    let mut gdb = Command::new("gdb");
    gdb.args([
        "-ex",
        format!("target remote {}", remote).as_str(),
        "-ex",
        format!("file {}", file_path.display()).as_str(),
    ]);
    gdb.status().unwrap();
}

#[test]
fn have_gdb_installed() {
    let output = Command::new("gdb").arg("--version").output();
    assert!(output.is_ok(), "Failed to run gdb");
    let stdout = String::from_utf8_lossy(&output.unwrap().stdout).to_string();
    assert!(stdout.contains("GNU gdb"));
}
