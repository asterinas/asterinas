// SPDX-License-Identifier: MPL-2.0

use std::process;

use super::util::{cargo, COMMON_CARGO_ARGS};
use crate::{commands::util::create_target_json, error::Errno, error_msg};

pub fn execute_check_command() {
    let target_json_path = create_target_json();

    let mut command = cargo();
    command.arg("check").arg("--target").arg(target_json_path);
    command.args(COMMON_CARGO_ARGS);
    let status = command.status().unwrap();
    if !status.success() {
        error_msg!("Check failed");
        process::exit(Errno::ExecuteCommand as _);
    }
}
