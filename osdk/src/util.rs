// SPDX-License-Identifier: MPL-2.0

use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{error::Errno, error_msg};

use quote::ToTokens;

/// FIXME: We should publish the asterinas crates to a public registry
/// and use the published version in the generated Cargo.toml.
pub const ASTER_GIT_LINK: &str = "https://github.com/asterinas/asterinas";
/// Make sure it syncs with the builder dependency in Cargo.toml.
pub const ASTER_GIT_REV: &str = "c9b66bd";
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

/// Retrieve the default member in the workspace.
///
/// If there is only one kernel crate, return that crate;
/// If there are multiple kernel crates or no kernel crates in the workspace,
/// this function will exit with an error.
///
/// A crate is considered a kernel crate if it utilizes the `aster_main` macro.
fn get_default_member(metadata: &serde_json::Value) -> &str {
    let default_members = metadata
        .get("workspace_default_members")
        .unwrap()
        .as_array()
        .unwrap();

    if default_members.len() == 1 {
        return default_members[0].as_str().unwrap();
    }

    let packages: Vec<_> = {
        let packages = metadata.get("packages").unwrap().as_array().unwrap();

        packages
            .iter()
            .filter(|package| {
                let id = package.get("id").unwrap();
                if !default_members.contains(id) {
                    return false;
                }

                let src_path = {
                    let targets = package.get("targets").unwrap().as_array().unwrap();
                    if targets.len() != 1 {
                        return false;
                    }
                    targets[0].get("src_path").unwrap().as_str().unwrap()
                };

                let file = {
                    let content = fs::read_to_string(src_path).unwrap();
                    syn::parse_file(&content).unwrap()
                };

                contains_aster_main_macro(&file)
            })
            .collect()
    };

    if packages.is_empty() {
        error_msg!("OSDK requires there's at least one kernel package. Please navigate to the kernel package directory or the workspace root and run the command.");
        std::process::exit(Errno::BuildCrate as _);
    }

    if packages.len() >= 2 {
        error_msg!("OSDK requires there's at most one kernel package in the workspace. Please navigate to the kernel package directory and run the command.");
        std::process::exit(Errno::BuildCrate as _);
    }

    packages[0].get("id").unwrap().as_str().unwrap()
}

fn contains_aster_main_macro(file: &syn::File) -> bool {
    for item in &file.items {
        let syn::Item::Fn(item_fn) = item else {
            continue;
        };

        for attr in &item_fn.attrs {
            let attr = format!("{}", attr.to_token_stream());
            if attr.as_str() == "# [aster_main]" {
                return true;
            }
        }
    }

    false
}

pub fn get_current_crate_info() -> CrateInfo {
    let metadata = get_cargo_metadata(None::<&str>, None::<&[&str]>).unwrap();

    let default_member = get_default_member(&metadata);
    // Prior 202403 (Rust 1.77.1), the default member string here is in the form of
    // "<crate_name> <crate_version> (path+file://<crate_path>)".
    // After that, it's
    // "path+file://<crate_path>#<crate_name>@<crate_version>", in which the crate
    // name might not exist if it is the last component of the path.
    if default_member.starts_with("path+file://") {
        // After 1.77.1
        if default_member.contains('@') {
            let default_member = default_member.split(['#', '@']).collect::<Vec<&str>>();
            CrateInfo {
                name: default_member[1].to_string(),
                version: default_member[2].to_string(),
                path: default_member[0]
                    .trim_start_matches("path+file://")
                    .to_string(),
            }
        } else {
            let default_member = default_member.split(['#']).collect::<Vec<&str>>();
            let path = default_member[0]
                .trim_start_matches("path+file://")
                .to_string();
            CrateInfo {
                name: PathBuf::from(path.clone())
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string(),
                version: default_member[1].to_string(),
                path,
            }
        }
    } else {
        // Before 1.77.1
        let default_member = default_member.split(' ').collect::<Vec<&str>>();
        CrateInfo {
            name: default_member[0].to_string(),
            version: default_member[1].to_string(),
            path: default_member[2]
                .trim_start_matches("(path+file://")
                .trim_end_matches(')')
                .to_string(),
        }
    }
}
