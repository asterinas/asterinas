// SPDX-License-Identifier: MPL-2.0

use std::process;

use super::util::{cargo, COMMON_CARGO_ARGS};
use crate::{error::Errno, error_msg};

pub fn execute_check_command() {
    let mut command = cargo();
    command
        .arg("check")
        .arg("--target")
        .arg("x86_64-unknown-none");
    command.args(COMMON_CARGO_ARGS);
    let status = command.status().unwrap();
    if !status.success() {
        error_msg!("Check failed");
        process::exit(Errno::ExecuteCommand as _);
    }
}
