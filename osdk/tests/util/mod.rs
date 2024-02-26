// SPDX-License-Identifier: MPL-2.0

//! The common utils for crate unit test

use std::{
    ffi::OsStr,
    fs::{self, create_dir_all},
    path::{Path, PathBuf},
    process::Output,
};

use assert_cmd::Command;

pub fn cargo_osdk<T: AsRef<OsStr>, I: IntoIterator<Item = T>>(args: I) -> Command {
    let mut command = Command::cargo_bin("cargo-osdk").unwrap();
    command.arg("osdk");
    command.args(args);
    command
}

pub fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "Command output {:#?} seems failed, stderr:\n {}",
        output,
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn assert_stdout_contains_msg(output: &Output, msg: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(msg))
}

pub fn create_workspace(workspace_name: &str, members: &[&str]) {
    // Create Cargo.toml
    let mut table = toml::Table::new();
    let workspace_table = {
        let mut table = toml::Table::new();
        table.insert("resolver".to_string(), toml::Value::String("2".to_string()));

        let members = members
            .iter()
            .map(|member| toml::Value::String(member.to_string()))
            .collect();

        let exclude = toml::Value::Array(vec![toml::Value::String("target/osdk/base".to_string())]);

        table.insert("members".to_string(), toml::Value::Array(members));
        table.insert("exclude".to_string(), exclude);
        table
    };

    table.insert("workspace".to_string(), toml::Value::Table(workspace_table));

    create_dir_all(workspace_name).unwrap();
    let manefest_path = PathBuf::from(workspace_name).join("Cargo.toml");

    let content = table.to_string();
    fs::write(manefest_path, content).unwrap();

    // Create OSDK.toml
    let osdk_manifest_path = PathBuf::from(workspace_name).join("OSDK.toml");
    fs::write(osdk_manifest_path, "").unwrap();

    // Create rust-toolchain.toml which is synced with the Asterinas' toolchain
    let rust_toolchain_path = PathBuf::from(workspace_name).join("rust-toolchain.toml");
    let content = include_str!("../../../rust-toolchain.toml");
    fs::write(rust_toolchain_path, content).unwrap();
}

pub fn add_member_to_workspace(workspace: impl AsRef<Path>, new_member: &str) {
    let path = PathBuf::from(workspace.as_ref()).join("Cargo.toml");

    let mut workspace_manifest: toml::Table = {
        let content = fs::read_to_string(&path).unwrap();
        toml::from_str(&content).unwrap()
    };

    let members = workspace_manifest
        .get_mut("workspace")
        .unwrap()
        .get_mut("members")
        .unwrap();
    if let toml::Value::Array(members) = members {
        members.push(toml::Value::String(new_member.to_string()));
    }

    let new_content = workspace_manifest.to_string();
    fs::write(&path, new_content).unwrap();
}
