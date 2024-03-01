// SPDX-License-Identifier: MPL-2.0

#![allow(unused)]

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::util::{assert_success, cargo_osdk, create_workspace};

// #[test]
fn build_with_default_manifest() {
    let workspace = "/tmp/workspace_foo";
    if Path::new(workspace).exists() {
        fs::remove_dir_all(workspace).unwrap();
    }

    let kernel_name: &str = "foo_os";
    create_workspace(workspace, &[kernel_name]);
    create_osdk_kernel(kernel_name, workspace);
    cargo_osdk_build(PathBuf::from(workspace).join(kernel_name), &[]);

    fs::remove_dir_all(workspace).unwrap();
}

// #[test]
fn build_with_conditional_manifest() {
    let workspace = "/tmp/workspace_bar";
    if Path::new(workspace).exists() {
        fs::remove_dir_all(workspace).unwrap();
    }

    let kernel_name: &str = "bar_os";
    create_workspace(workspace, &[kernel_name]);
    create_osdk_kernel_with_features(kernel_name, &["intel_tdx", "iommu"], workspace);
    let contents = include_str!("OSDK.toml.conditional");
    let path = PathBuf::from(workspace).join("OSDK.toml");
    fs::write(path, contents).unwrap();

    cargo_osdk_build(
        PathBuf::from(workspace).join(kernel_name),
        &["--profile", "release", "--features", "iommu"],
    );

    fs::remove_dir_all(workspace).unwrap();
}

fn create_osdk_kernel(name: &str, current_dir: &str) {
    let output = cargo_osdk(&["new", "--kernel", name])
        .current_dir(current_dir)
        .output()
        .unwrap();
    assert_success(&output);
}

fn create_osdk_kernel_with_features(name: &str, features: &[&str], current_dir: &str) {
    create_osdk_kernel(name, current_dir);
    let manifest_path = PathBuf::from(current_dir).join(name).join("Cargo.toml");
    let contents = fs::read_to_string(&manifest_path).unwrap();
    let mut manifest: toml::Table = toml::from_str(&contents).unwrap();

    let mut features_table = toml::Table::new();
    for feature in features {
        features_table.insert(feature.to_string(), toml::Value::Array(Vec::new()));
    }
    manifest.insert("features".to_string(), toml::Value::Table(features_table));

    fs::write(&manifest_path, manifest.to_string()).unwrap();
}

fn cargo_osdk_build<P: AsRef<Path>>(current_dir: P, args: &[&str]) {
    let mut command = cargo_osdk(&["build"]);
    command.args(args);
    command.current_dir(current_dir);
    let output = command.output().unwrap();
    assert_success(&output);
}
