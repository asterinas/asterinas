// SPDX-License-Identifier: MPL-2.0

//! This module contains subcommands of cargo-osdk.

mod build;
mod new;
mod run;
mod test;
mod util;

pub use self::{
    build::execute_build_command, new::execute_new_command, run::execute_run_command,
    test::execute_test_command,
};

use crate::target::get_default_target;

/// Execute the forwarded cargo command with args containing the subcommand and its arguments.
pub fn execute_forwarded_command(subcommand: &str, args: &Vec<String>) -> ! {
    let mut cargo = util::cargo();
    cargo
        .arg(subcommand)
        .args(util::COMMON_CARGO_ARGS)
        .arg("--target")
        .arg(get_default_target().triple())
        .args(args);
    let status = cargo.status().expect("Failed to execute cargo");
    std::process::exit(status.code().unwrap_or(1));
}
