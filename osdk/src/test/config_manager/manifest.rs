// SPDX-License-Identifier: MPL-2.0

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    cli::CargoArgs,
    config_manager::{
        get_feature_strings,
        manifest::{OsdkManifest, TomlManifest, SELECT_REGEX},
    },
    test::utils::{assert_success, cargo_osdk, create_workspace},
};

#[test]
fn deserialize_osdk_manifest() {
    let content = include_str!("OSDK.toml.empty");
    let osdk_manifest: TomlManifest = toml::from_str(content).unwrap();
    assert!(osdk_manifest == TomlManifest::default());

    let content = include_str!("OSDK.toml.full");
    let osdk_manifest: TomlManifest = toml::from_str(content).unwrap();
    assert!(osdk_manifest.boot.grub_mkrescue.unwrap() == PathBuf::from("/usr/bin/grub-mkrescue"));
}

#[test]
fn serialize_osdk_manifest() {
    let manifest = TomlManifest::default();
    let contents = toml::to_string(&manifest).unwrap();
    fs::write("OSDK.toml", contents).unwrap();
    fs::remove_file("OSDK.toml").unwrap();
}

#[test]
fn deserialize_conditional_osdk_manifest() {
    let content = include_str!("OSDK.toml.conditional");
    let manifest: TomlManifest = toml::from_str(content).unwrap();
    println!("manifest = {:?}", manifest);
}

#[test]
fn load_manifest() {
    let workspace = "workspace_foo";
    let kernel_name: &str = "foo_os";
    create_workspace(workspace, &[kernel_name]);
    create_osdk_kernel(kernel_name, workspace);
    let cargo_args = CargoArgs::default();
    cargo_osdk_build(PathBuf::from(workspace).join(kernel_name), &cargo_args);
    fs::remove_dir_all(workspace).unwrap();
}

#[test]
fn load_manifest_conditional() {
    let workspace = "workspace_bar";
    let kernel_name: &str = "bar_os";
    create_workspace(workspace, &[kernel_name]);
    create_osdk_kernel_with_features(kernel_name, &["intel_tdx", "iommu"], workspace);
    let contents = include_str!("OSDK.toml.conditional");
    let path = PathBuf::from(workspace).join("OSDK.toml");
    fs::write(path, contents).unwrap();

    let cargo_args = CargoArgs {
        profile: "release".to_string(),
        features: vec![String::from("iommu")],
    };
    cargo_osdk_build(PathBuf::from(workspace).join(kernel_name), &cargo_args);

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

fn cargo_osdk_build<P: AsRef<Path>>(current_dir: P, cargo_args: &CargoArgs) {
    let args = get_feature_strings(cargo_args);
    let mut command = cargo_osdk(&["build"]);
    command.args(args);
    command.current_dir(current_dir);
    let output = command.output().unwrap();
    assert_success(&output);
}

#[test]
fn conditional_manifest() {
    let toml_manifest: TomlManifest = {
        let content = include_str!("OSDK.toml.conditional");
        toml::from_str(content).unwrap()
    };

    assert!(toml_manifest.qemu.cfg.is_some());
    assert!(toml_manifest
        .qemu
        .cfg
        .as_ref()
        .unwrap()
        .contains_key(&String::from("cfg(select=\"intel_tdx\")")));
    assert!(toml_manifest
        .qemu
        .cfg
        .as_ref()
        .unwrap()
        .contains_key(&String::from("cfg(select=\"iommu\")")));

    // Default selection
    let selection: Option<&str> = None;
    let manifest = OsdkManifest::from_toml_manifest(toml_manifest.clone(), selection);
    assert!(manifest.qemu.args.contains(&String::from(
        "-device virtio-keyboard-pci,disable-legacy=on,disable-modern=off"
    )));

    // Iommu
    let selection: Option<&str> = Some("iommu");
    let manifest = OsdkManifest::from_toml_manifest(toml_manifest.clone(), selection);
    assert!(manifest
        .qemu
        .args
        .contains(&String::from("-device ioh3420,id=pcie.0,chassis=1")));

    // Tdx
    let selection: Option<&str> = Some("intel_tdx");
    let manifest = OsdkManifest::from_toml_manifest(toml_manifest.clone(), selection);
    assert!(manifest.qemu.args.is_empty());
}

#[test]
fn extract_selection() {
    let text = "cfg(select=\"abc123_\")";
    let captures = SELECT_REGEX.captures(text).unwrap();
    let selection = captures.name("select").unwrap().as_str();
    assert_eq!(selection, "abc123_");
}
