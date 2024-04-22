// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::config_manager::cfg::Cfg;

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
fn test_cfg_from_str() {
    let cfg = Cfg::from([("arch", "x86_64"), ("select", "foo")]);
    let cfg1 = Cfg::from_str(" cfg(arch =  \"x86_64\",     select=\"foo\", )").unwrap();
    let cfg2 = Cfg::from_str("cfg(arch=\"x86_64\",select=\"foo\")").unwrap();
    let cfg3 = Cfg::from_str(" cfg( arch=\"x86_64\", select=\"foo\" )").unwrap();
    assert_eq!(cfg, cfg1);
    assert_eq!(cfg, cfg2);
    assert_eq!(cfg, cfg3);
}

#[test]
fn test_cfg_display() {
    let cfg = Cfg::from([("arch", "x86_64"), ("select", "foo")]);
    let cfg_string = cfg.to_string();
    let cfg_back = Cfg::from_str(&cfg_string).unwrap();
    assert_eq!(cfg_string, "cfg(arch=\"x86_64\", select=\"foo\")");
    assert_eq!(cfg, cfg_back);
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

    assert!(toml_manifest.qemu.cfg_map.is_some());
    assert!(toml_manifest
        .qemu
        .cfg_map
        .as_ref()
        .unwrap()
        .contains_key(&Cfg::from([("arch", "x86_64"), ("select", "intel_tdx")])));
    assert!(toml_manifest
        .qemu
        .cfg_map
        .as_ref()
        .unwrap()
        .contains_key(&Cfg::from([("select", "iommu")])));

    // Default selection
    let arch = None;
    let selection: Option<String> = None;
    let manifest = OsdkManifest::from_toml_manifest(toml_manifest.clone(), arch, selection);
    assert!(manifest.qemu.args.contains(&String::from(
        "-device virtio-keyboard-pci,disable-legacy=on,disable-modern=off"
    )));

    // Iommu
    let arch = None;
    let selection: Option<String> = Some("iommu".to_owned());
    let manifest = OsdkManifest::from_toml_manifest(toml_manifest.clone(), arch, selection);
    assert!(manifest
        .qemu
        .args
        .contains(&String::from("-device ioh3420,id=pcie.0,chassis=1")));

    // Tdx
    let arch = Some("x86_64".to_owned());
    let selection: Option<String> = Some("intel_tdx".to_owned());
    let manifest = OsdkManifest::from_toml_manifest(toml_manifest.clone(), arch, selection);
    assert!(manifest.qemu.args.is_empty());
}
