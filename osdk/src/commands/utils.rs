// SPDX-License-Identifier: MPL-2.0

use std::{fs, path::PathBuf, process::Command};

use crate::utils::get_target_directory;

pub const COMMON_CARGO_ARGS: &[&str] = &[
    "-Zbuild-std=core,alloc,compiler_builtins",
    "-Zbuild-std-features=compiler-builtins-mem",
];

pub const DEFAULT_TARGET_RELPATH: &str = "osdk";

pub fn cargo() -> Command {
    Command::new("cargo")
}

pub fn create_target_json() -> PathBuf {
    let target_osdk_dir = get_target_directory().join(DEFAULT_TARGET_RELPATH);
    fs::create_dir_all(&target_osdk_dir).unwrap();

    let target_json_path = target_osdk_dir.join("x86_64-custom.json");
    if target_json_path.is_file() {
        return target_json_path;
    }

    let contents = include_str!("../base_crate/x86_64-custom.json.template");
    fs::write(&target_json_path, contents).unwrap();

    target_json_path
}
