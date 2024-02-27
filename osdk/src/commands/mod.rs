// SPDX-License-Identifier: MPL-2.0

//! This module contains subcommands of cargo-osdk.

mod build;
mod check;
mod clippy;
mod new;
mod run;
mod test;
mod util;

pub use self::{
    build::execute_build_command, check::execute_check_command, clippy::execute_clippy_command,
    new::execute_new_command, run::execute_run_command, test::execute_test_command,
};
