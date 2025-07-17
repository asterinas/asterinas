// SPDX-License-Identifier: MPL-2.0

//! The common utils for crate unit test

use std::{
    ffi::OsStr,
    fs::{self, create_dir_all, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Output,
};

use assert_cmd::Command;
use toml::{Table, Value};

pub fn cargo_osdk<T: AsRef<OsStr>, I: IntoIterator<Item = T> + Copy>(args: I) -> Command {
    let mut command = Command::cargo_bin("cargo-osdk").unwrap();
    command.arg("osdk");
    command.args(args);
    conditionally_add_tdx_args(&mut command, args);
    command
}

pub fn edit_config_files(dir: &Path) {
    let manifest_path = dir.join("Cargo.toml");
    assert!(manifest_path.is_file());
    depends_on_local_ostd(manifest_path);
    if is_tdx_enabled() {
        let osdk_path = dir.join("OSDK.toml");
        add_tdx_scheme(osdk_path).unwrap();
    };
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

        let exclude = toml::Value::Array(vec![
            toml::Value::String("target/osdk/base".to_string()),
            toml::Value::String("target/osdk/test-base".to_string()),
        ]);

        table.insert("members".to_string(), toml::Value::Array(members));
        table.insert("exclude".to_string(), exclude);
        table
    };

    table.insert("workspace".to_string(), toml::Value::Table(workspace_table));

    create_dir_all(workspace_name).unwrap();
    let manefest_path = PathBuf::from(workspace_name).join("Cargo.toml");

    let content = table.to_string();
    fs::write(manefest_path, content).unwrap();

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

/// Makes crates created by `cargo ostd new` depends on ostd locally,
/// instead of ostd from remote source(git repo/crates.io).
///
/// Each crate created by `cargo ostd new` should add this patch.
pub(crate) fn depends_on_local_ostd(manifest_path: impl AsRef<Path>) {
    let crate_dir = env!("CARGO_MANIFEST_DIR");
    let ostd_dir = PathBuf::from(crate_dir)
        .join("..")
        .join("ostd")
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();

    // FIXME: It may be more elegant to add `patch` section instead of replacing dependency.
    // But adding `patch` section does not work in my local test, which is confusing.

    let manifest_content = fs::read_to_string(&manifest_path).unwrap();
    let mut manifest: Table = toml::from_str(&manifest_content).unwrap();
    let dep = manifest
        .get_mut("dependencies")
        .map(Value::as_table_mut)
        .flatten()
        .unwrap();

    let mut table = Table::new();
    table.insert("path".to_string(), Value::String(ostd_dir));
    dep.insert("ostd".to_string(), Value::Table(table));

    fs::write(manifest_path, manifest.to_string().as_bytes()).unwrap();
}

/// Makes crates created by `cargo ostd new` enable coverage features,
/// adding features = ["coverage"] to ostd dependency and creating a [features] section.
///
/// This transforms the manifest to enable coverage support.
pub(crate) fn depends_on_coverage(manifest_path: impl AsRef<Path>, osdk_path: impl AsRef<Path>) {
    let manifest_content = fs::read_to_string(&manifest_path).unwrap();
    let mut manifest: Table = toml::from_str(&manifest_content).unwrap();

    // Add features = ["coverage"] to ostd dependency
    let dep = manifest
        .get_mut("dependencies")
        .map(Value::as_table_mut)
        .flatten()
        .unwrap();

    if let Some(ostd_dep) = dep.get_mut("ostd").map(Value::as_table_mut).flatten() {
        let features = vec![Value::String("coverage".to_string())];
        ostd_dep.insert("features".to_string(), Value::Array(features));
    }

    // Add [features] section with coverage = []
    let mut features_table = Table::new();
    features_table.insert("coverage".to_string(), Value::Array(vec![]));
    manifest.insert("features".to_string(), Value::Table(features_table));

    fs::write(manifest_path, manifest.to_string().as_bytes()).unwrap();

    // Modify OSDK.toml to add logfile=qemu.log to chardev line
    if osdk_path.as_ref().exists() {
        let osdk_content = fs::read_to_string(&osdk_path).unwrap();
        let modified_content = osdk_content.replace(
            "-chardev stdio,id=mux,mux=on,signal=off \\",
            "-chardev stdio,id=mux,mux=on,signal=off,logfile=qemu.log \\",
        );
        fs::write(osdk_path, modified_content).unwrap();
    }
}

pub(crate) fn add_tdx_scheme(osdk_path: impl AsRef<Path>) -> std::io::Result<()> {
    let template_path = Path::new(file!())
        .parent()
        .unwrap()
        .join("scheme.tdx.template");
    let mut file = OpenOptions::new()
        .write(true)
        .append(true)
        .open(osdk_path)?;
    let tdx_qemu_cfg = fs::read_to_string(template_path)?;
    file.write_all(format!("\n\n{}", tdx_qemu_cfg).as_bytes())?;
    Ok(())
}

pub(crate) fn is_tdx_enabled() -> bool {
    std::env::var("INTEL_TDX").is_ok_and(|s| s == "1")
}

fn conditionally_add_tdx_args<T: AsRef<OsStr>, I: IntoIterator<Item = T> + Copy>(
    command: &mut Command,
    args: I,
) {
    if is_tdx_enabled() && contains_build_run_or_test(args) {
        command.args(&["--scheme", "tdx"]);
    }
}

fn contains_build_run_or_test<T: AsRef<OsStr>, I: IntoIterator<Item = T>>(args: I) -> bool {
    args.into_iter().any(|arg| {
        if let Some(arg_str) = arg.as_ref().to_str() {
            arg_str == "build" || arg_str == "run" || arg_str == "test"
        } else {
            false
        }
    })
}
