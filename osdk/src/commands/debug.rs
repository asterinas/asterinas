// SPDX-License-Identifier: MPL-2.0

use crate::{
    cli::DebugArgs,
    commands::util::bin_file_name,
    util::{get_kernel_crate, get_target_directory, new_command_checked_exists},
};

pub fn execute_debug_command(_profile: &str, args: &DebugArgs) {
    let remote = &args.remote;

    let file_path = get_target_directory()
        .join("osdk")
        .join(get_kernel_crate().name)
        .join(bin_file_name());
    println!("Debugging {}", file_path.display());

    let mut gdb = new_command_checked_exists("gdb");
    gdb.args([
        format!("{}", file_path.display()).as_str(),
        "-ex",
        format!("target remote {}", remote).as_str(),
    ]);
    gdb.status().unwrap();
}

#[test]
fn have_gdb_installed() {
    let output = new_command_checked_exists("gdb").arg("--version").output();
    assert!(output.is_ok(), "Failed to run gdb");
    let stdout = String::from_utf8_lossy(&output.unwrap().stdout).to_string();
    assert!(stdout.contains("GNU gdb"));
}
