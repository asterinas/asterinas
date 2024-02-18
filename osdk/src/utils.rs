// SPDX-License-Identifier: MPL-2.0

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use crate::error::Errno;
use crate::error_msg;

// FIXME: Crates belonging to Asterinas require a different dependency format. The dependency 
// should be specified using a relative path instead of a URL.
pub const ASTER_FRAME_DEP: &str =
    "aster-frame = { git = \"https://github.com/asterinas/asterinas\", rev = \"f2f991b\" }";

fn cargo() -> Command {
    Command::new("cargo")
}

/// Create a new library crate with cargo
pub fn cargo_new_lib(crate_name: &str) {
    let mut command = cargo();
    command.args(["new", "--lib", crate_name]);
    let status = command.status().unwrap();
    if !status.success() {
        error_msg!("Failed to create new crate");
        std::process::exit(Errno::CreateCrate as _);
    }
}

pub fn get_cargo_metadata<S1: AsRef<Path>, S2: AsRef<OsStr>>(
    current_dir: Option<S1>,
    cargo_args: Option<&[S2]>,
) -> serde_json::Value {
    let mut command = cargo();
    command.args(["metadata", "--no-deps", "--format-version", "1"]);

    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }

    if let Some(cargo_args) = cargo_args {
        command.args(cargo_args);
    }

    let output = command.output().unwrap();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("{}", &stderr);

        error_msg!("Failed to get metadata for newly created crate");
        std::process::exit(Errno::GetMetadata as _);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).unwrap()
}
