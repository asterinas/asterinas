// SPDX-License-Identifier: MPL-2.0

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const COMMON_CARGO_ARGS: &[&str] = &[
    "-Zbuild-std=core,alloc,compiler_builtins",
    "-Zbuild-std-features=compiler-builtins-mem",
];

pub fn cargo() -> Command {
    Command::new("cargo")
}

pub fn create_target_json(target_directory: impl AsRef<Path>) -> PathBuf {
    let target_osdk_dir = PathBuf::from(target_directory.as_ref()).join("osdk");
    fs::create_dir_all(&target_osdk_dir).unwrap();

    let target_json_path = target_osdk_dir.join("x86_64-custom.json");
    if target_json_path.is_file() {
        return target_json_path;
    }

    let contents = include_str!("template/x86_64-custom.json.template");
    fs::write(&target_json_path, contents).unwrap();

    target_json_path
}
