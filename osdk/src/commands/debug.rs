// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

use crate::{
    cli::DebugArgs,
    commands::util::bin_file_name,
    util::{
        get_cargo_metadata, get_kernel_crate, get_target_directory, new_command_checked_exists,
    },
};

pub fn execute_debug_command(_profile: &str, args: &DebugArgs) {
    let remote = &args.remote;

    let file_path = get_target_directory()
        .join("osdk")
        .join(get_kernel_crate().name)
        .join(bin_file_name());
    println!("Debugging {}", file_path.display());

    let mut gdb = new_command_checked_exists("rust-gdb");
    gdb.args([
        format!("{}", file_path.display()).as_str(),
        "-ex",
        format!("target remote {}", remote).as_str(),
    ]);

    if let Some(helper_script) = asterinas_gdb_script() {
        let source_cmd = format!("source {}", helper_script.display());
        gdb.args(["-ex", &source_cmd]);
    }

    gdb.status().unwrap();
}

#[test]
fn have_rust_gdb_installed() {
    let output = new_command_checked_exists("rust-gdb")
        .arg("--version")
        .output();
    assert!(output.is_ok(), "Failed to run rust-gdb");
    let stdout = String::from_utf8_lossy(&output.unwrap().stdout).to_string();
    assert!(stdout.contains("GNU gdb"));
}

fn asterinas_gdb_script() -> Option<PathBuf> {
    let metadata = get_cargo_metadata(None::<&str>, None::<&[&str]>)?;
    let workspace_root = metadata.get("workspace_root")?.as_str()?;
    let script = PathBuf::from(workspace_root)
        .join("scripts")
        .join("gdb")
        .join("asterinas-gdb.py");
    script.exists().then_some(script)
}
