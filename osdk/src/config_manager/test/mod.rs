// SPDX-License-Identifier: MPL-2.0

use super::*;

#[test]
fn deserialize_toml_manifest() {
    let content = include_str!("OSDK.toml.full");
    let toml_manifest: TomlManifest = toml::from_str(content).unwrap();
    assert!(toml_manifest.project.type_ == manifest::ProjectType::Kernel);
}

#[test]
fn conditional_manifest() {
    let toml_manifest: TomlManifest = {
        let content = include_str!("OSDK.toml.full");
        toml::from_str(content).unwrap()
    };
    let arch = crate::arch::Arch::X86_64;

    // Default schema
    let schema: Option<String> = None;
    let manifest = toml_manifest.get_osdk_manifest(PathBuf::from("/"), arch, schema);
    assert!(manifest.run.unwrap().qemu_args.contains(&String::from(
        "-device virtio-blk-pci,bus=pcie.0,addr=0x7,drive=x1,serial=vexfat,disable-legacy=on,disable-modern=off",
    )));

    // Iommu
    let schema: Option<String> = Some("iommu".to_owned());
    let manifest = toml_manifest.get_osdk_manifest(PathBuf::from("/"), arch, schema);
    assert!(manifest
        .run
        .unwrap()
        .qemu_args
        .contains(&String::from("-device ioh3420,id=pcie.0,chassis=1")));

    // Tdx
    let schema: Option<String> = Some("intel_tdx".to_owned());
    let manifest = toml_manifest.get_osdk_manifest(PathBuf::from("/"), arch, schema);
    assert_eq!(
        manifest.run.unwrap().qemu_exe.unwrap(),
        PathBuf::from("/usr/bin/bash")
    );
}
