// SPDX-License-Identifier: MPL-2.0

//! This module contains subcommands of cargo-osdk.

mod build;
mod debug;
mod new;
mod run;
mod test;
mod util;

pub use self::{
    build::execute_build_command, debug::execute_debug_command, new::execute_new_command,
    run::execute_run_command, test::execute_test_command,
};

use crate::arch::get_default_arch;

/// Execute the forwarded cargo command with args containing the subcommand and its arguments.
pub fn execute_forwarded_command(subcommand: &str, args: &Vec<String>) -> ! {
    let mut cargo = util::cargo();
    cargo.arg(subcommand).args(util::COMMON_CARGO_ARGS);
    if !args.contains(&"--target".to_owned()) {
        cargo.arg("--target").arg(get_default_arch().triple());
    }
    cargo.args(args);

    let env_rustflags = std::env::var("RUSTFLAGS").unwrap_or_default();
    let rustflags = env_rustflags + " --check-cfg cfg(ktest)";

    cargo.env("RUSTFLAGS", rustflags);

    let status = cargo.status().expect("Failed to execute cargo");
    std::process::exit(status.code().unwrap_or(1));
}
