// SPDX-License-Identifier: MPL-2.0

use clap::ValueEnum;

use super::util;
use crate::arch::Arch;

/// Execute the build cargo command for each architecture without arguments.
pub fn execute_fetch_command(args: &[String]) {
    let target_archs: Vec<&str> = Arch::value_variants()
        .iter()
        .map(|arch| arch.to_str())
        .collect();

    for current_arch in target_archs {
        let mut cargo = util::cargo();
        cargo
            .arg("osdk")
            .arg("build")
            .arg(format!("--target-arch={}", current_arch))
            .args(
                args.iter()
                    .filter(|&arg| !(arg.starts_with("--target-arch=") || arg == "--offline")),
            );
        cargo.status().expect("Failed to execute cargo");
    }
}
