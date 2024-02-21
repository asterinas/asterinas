// SPDX-License-Identifier: MPL-2.0

use std::process;

use crate::commands::utils::create_target_json;
use crate::error::Errno;
use crate::error_msg;

use super::utils::{cargo, COMMON_CARGO_ARGS};
use crate::{
    commands::utils::create_target_json, error::Errno, error_msg, utils::get_cargo_metadata,
};

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
