// SPDX-License-Identifier: MPL-2.0

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{error::Errno, error_msg};

/// FIXME: We should publish the asterinas crates to a public registry
/// and use the published version in the generated Cargo.toml.
pub const ASTER_GIT_LINK: &str = "https://github.com/asterinas/asterinas";
pub const ASTER_GIT_REV: &str = "7d0ea99";
pub fn aster_crate_dep(crate_name: &str) -> String {
    format!(
        "{} = {{ git = \"{}\", rev = \"{}\" }}",
        crate_name, ASTER_GIT_LINK, ASTER_GIT_REV
    )
}

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

/// Get the Cargo metadata parsed from the standard output
/// of the invocation of Cargo. Return `None` if the command
/// fails or the `current_dir` is not in a Cargo workspace.
pub fn get_cargo_metadata<S1: AsRef<Path>, S2: AsRef<OsStr>>(
    current_dir: Option<S1>,
    cargo_args: Option<&[S2]>,
) -> Option<serde_json::Value> {
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
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Some(serde_json::from_str(&stdout).unwrap())
}

pub fn get_target_directory() -> PathBuf {
    let metadata = get_cargo_metadata(None::<&str>, None::<&[&str]>).unwrap();
    metadata
        .get("target_directory")
        .unwrap()
        .as_str()
        .unwrap()
        .into()
}

pub struct CrateInfo {
    pub name: String,
    pub version: String,
    pub path: String,
}

pub fn get_current_crate_info() -> CrateInfo {
    let metadata = get_cargo_metadata(None::<&str>, None::<&[&str]>).unwrap();
    let default_members = metadata.get("workspace_default_members").unwrap();
    assert_eq!(default_members.as_array().unwrap().len(), 1);
    // The default member string here is in the form of "<crate_name> <crate_version> (path+file://<crate_path>)"
    let default_member = default_members[0]
        .as_str()
        .unwrap()
        .split(' ')
        .collect::<Vec<&str>>();
    let name = default_member[0].to_string();
    let version = default_member[1].to_string();
    let path = default_member[2]
        .trim_start_matches("(path+file://")
        .trim_end_matches(')')
        .to_string();
    CrateInfo {
        name,
        version,
        path,
    }
}
