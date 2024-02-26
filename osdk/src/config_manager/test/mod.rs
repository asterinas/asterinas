// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn split_kcmd_args_test() {
    let mut kcmd_args = ["init=/bin/sh", "--", "sh", "-l"]
        .iter()
        .map(ToString::to_string)
        .collect();
    let init_args = split_kcmd_args(&mut kcmd_args);
    let expected_kcmd_args: Vec<_> = ["init=/bin/sh"].iter().map(ToString::to_string).collect();
    assert_eq!(kcmd_args, expected_kcmd_args);
    let expecetd_init_args: Vec<_> = ["sh", "-l"].iter().map(ToString::to_string).collect();
    assert_eq!(init_args, expecetd_init_args);

    let mut kcmd_args = ["init=/bin/sh", "--"]
        .iter()
        .map(ToString::to_string)
        .collect();
    let init_args = split_kcmd_args(&mut kcmd_args);
    let expected_kcmd_args: Vec<_> = ["init=/bin/sh"].iter().map(ToString::to_string).collect();
    assert_eq!(kcmd_args, expected_kcmd_args);
    let expecetd_init_args: Vec<String> = Vec::new();
    assert_eq!(init_args, expecetd_init_args);

    let mut kcmd_args = ["init=/bin/sh", "shell=/bin/sh"]
        .iter()
        .map(ToString::to_string)
        .collect();
    let init_args = split_kcmd_args(&mut kcmd_args);
    let expected_kcmd_args: Vec<_> = ["init=/bin/sh", "shell=/bin/sh"]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(kcmd_args, expected_kcmd_args);
    let expecetd_init_args: Vec<String> = Vec::new();
    assert_eq!(init_args, expecetd_init_args);
}

#[test]
fn get_key_test() {
    let string1 = "init=/bin/init";
    let key = get_key(string1, "=").unwrap();
    assert_eq!(key.as_str(), "init");

    let string2 = "-m 2G";
    let key = get_key(string2, " ").unwrap();
    assert_eq!(key.as_str(), "-m");

    let string3 = "-device virtio-keyboard-pci,disable-legacy=on,disable-modern=off";
    let key = get_key(string3, " ").unwrap();
    assert_eq!(key.as_str(), "-device");

    let string4 = "-device";
    assert!(get_key(string4, " ").is_none());
}

#[test]
fn apply_kv_array_test() {
    let qemu_args = &[
        "-enable-kvm",
        "-m 2G",
        "-device virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,disable-legacy=on,disable-modern=off",
        "-device virtio-keyboard-pci,disable-legacy=on,disable-modern=off",
    ];

    let args = &["-m 100G", "-device ioh3420,id=pcie.0,chassis=1"];

    let expected = &[
        "-enable-kvm",
        "-m 100G",
        "-device virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,disable-legacy=on,disable-modern=off",
        "-device virtio-keyboard-pci,disable-legacy=on,disable-modern=off",
        "-device ioh3420,id=pcie.0,chassis=1",
    ];

    let mut array = qemu_args.iter().map(ToString::to_string).collect();
    let args = args.iter().map(ToString::to_string).collect();
    apply_kv_array(&mut array, &args, " ", &["-device"]);

    let expected: Vec<_> = expected.iter().map(ToString::to_string).collect();
    assert_eq!(expected, array);
}

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
