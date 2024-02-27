// SPDX-License-Identifier: MPL-2.0

use std::process;

use super::util::{cargo, COMMON_CARGO_ARGS};
use crate::{commands::util::create_target_json, error::Errno, error_msg};

pub fn execute_clippy_command() {
    let target_json_path = create_target_json();

    let mut command = cargo();
    command.arg("clippy").arg("-h");
    info!("Running `cargo clippy -h`");
    let output = command.output().unwrap();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("{}", &stderr);
        error_msg!("Cargo clippy failed");
        process::exit(Errno::ExecuteCommand as _);
    }

    let mut command = cargo();
    command.arg("clippy").arg("--target").arg(target_json_path);
    command.args(COMMON_CARGO_ARGS);
    // TODO: Add support for custom clippy args using OSDK commandline rather than hardcode it.
    command.args(["--", "-D", "warnings"]);
    let status = command.status().unwrap();
    if !status.success() {
        error_msg!("Cargo clippy failed");
        process::exit(Errno::ExecuteCommand as _);
    }
}
