// SPDX-License-Identifier: MPL-2.0

//! This module contains subcommands of cargo-osdk.

mod build;
mod debug;
mod new;
mod profile;
mod run;
mod test;
mod util;

pub use self::{
    build::execute_build_command, debug::execute_debug_command, new::execute_new_command,
    profile::execute_profile_command, run::execute_run_command, test::execute_test_command,
};

use crate::{
    arch::get_default_arch,
    error_msg,
    util::{get_current_crates, DirGuard},
};

/// Execute the forwarded cargo command with arguments.
///
/// The `cfg_ktest` parameter controls whether `cfg(ktest)` is enabled.
pub fn execute_forwarded_command(subcommand: &str, args: &Vec<String>, cfg_ktest: bool) {
    let mut cargo = util::cargo();
    cargo.arg(subcommand).args(util::COMMON_CARGO_ARGS);
    if !args.contains(&"--target".to_owned()) {
        cargo.arg("--target").arg(get_default_arch().triple());
    }
    cargo.args(args);

    let env_rustflags = std::env::var("RUSTFLAGS").unwrap_or_default();
    let rustflags = env_rustflags + " --check-cfg cfg(ktest)";
    let rustflags = if cfg_ktest {
        rustflags + " --cfg ktest"
    } else {
        rustflags
    };

    cargo.env("RUSTFLAGS", rustflags);

    // When generating documentation via `cargo doc`, the `--check-cfg cfg(ktest)` flag
    // must be specified in both `RUSTFLAGS` and `RUSTDOCFLAGS`.
    if subcommand == "doc" {
        let env_rustdocflags = std::env::var("RUSTDOCFLAGS").unwrap_or_default();
        let rustdocflags = env_rustdocflags + " --check-cfg cfg(ktest)";
        cargo.env("RUSTDOCFLAGS", rustdocflags);
    }

    let status = cargo.status().expect("Failed to execute cargo");
    if !status.success() {
        error_msg!("Command {:?} failed with status: {:?}", cargo, status);
        std::process::exit(status.code().unwrap_or(1));
    }
}

/// Execute the forwarded cargo command on each crate in the workspace.
///
/// It works like invoking [`execute_forwarded_command`] on each crate in the
/// workspace, but it creates a base crate that depends on the target crate and
/// executes the command on the base crate if a target crate is a kernel crate.
///
/// It invokes Cargo on the base crate only if the target crate is a kernel
/// crate. Otherwise, it behaves just like [`execute_forwarded_command`].
pub fn execute_forwarded_command_on_each_crate(
    subcommand: &str,
    args: &Vec<String>,
    cfg_ktest: bool,
) {
    let target_crates = get_current_crates();
    for target in target_crates {
        let _dir_guard = DirGuard::change_dir(target.path);
        execute_forwarded_command(subcommand, args, cfg_ktest);
    }
}
