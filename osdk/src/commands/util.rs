// SPDX-License-Identifier: MPL-2.0

use std::process::Command;

use crate::util::{get_cargo_metadata, get_current_crate_info, parse_package_id_string};

pub const COMMON_CARGO_ARGS: &[&str] = &[
    "-Zbuild-std=core,alloc,compiler_builtins",
    "-Zbuild-std-features=compiler-builtins-mem",
];

pub const DEFAULT_TARGET_RELPATH: &str = "osdk";
pub const DEFAULT_MIRI_TARGET_RELPATH: &str = "osdk-miri";

pub fn cargo() -> Command {
    Command::new("cargo")
}

pub fn profile_name_adapter(profile: &str) -> &str {
    match profile {
        "dev" => "debug",
        _ => profile,
    }
}

pub fn bin_file_name() -> String {
    get_current_crate_info().name + "-osdk-bin"
}

pub fn get_workspace_default_members() -> Vec<String> {
    let metadata = get_cargo_metadata(None::<&str>, None::<&[&str]>).unwrap();
    let default_members = metadata
        .get("workspace_default_members")
        .unwrap()
        .as_array()
        .unwrap();
    default_members
        .iter()
        .map(|value| {
            let default_member = value.as_str().unwrap();
            let crate_info = parse_package_id_string(default_member);
            crate_info.path
        })
        .collect()
}
