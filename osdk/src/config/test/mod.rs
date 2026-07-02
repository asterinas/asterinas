// SPDX-License-Identifier: MPL-2.0

use std::fs::{self, File};

use super::*;
use crate::config::scheme::BootMethod;

#[test]
fn deserialize_toml_manifest() {
    let content = include_str!("OSDK.toml.full");
    let toml_manifest: manifest::TomlManifest = toml::from_str(content).unwrap();
    let type_ = toml_manifest.project_type.unwrap();
    assert!(type_ == manifest::ProjectType::Kernel);
}

#[test]
fn conditional_manifest() {
    let tmp_file = "/tmp/osdk_test_file";
    File::create(tmp_file).unwrap();

    let toml_manifest: manifest::TomlManifest = {
        let content = include_str!("OSDK.toml.full");
        toml::from_str(content).unwrap()
    };

    // Default scheme
    let scheme = toml_manifest.get_scheme(None::<String>);
    assert!(
        scheme
            .qemu
            .as_ref()
            .unwrap()
            .args
            .as_ref()
            .unwrap()
            .contains(&String::from("-machine q35",))
    );

    // Iommu
    let scheme = toml_manifest.get_scheme(Some("iommu".to_owned()));
    assert!(
        scheme
            .qemu
            .as_ref()
            .unwrap()
            .args
            .as_ref()
            .unwrap()
            .contains(&String::from("-device ioh3420,id=pcie.0,chassis=1",))
    );

    // Linux64 direct
    let scheme = toml_manifest.get_scheme(Some("directvm".to_owned()));
    assert_eq!(
        scheme.boot.as_ref().unwrap().method,
        Some(BootMethod::Linux64Direct)
    );
    assert!(scheme.build.as_ref().unwrap().strip_elf);

    // Tdx
    let scheme = toml_manifest.get_scheme(Some("tdx".to_owned()));
    assert_eq!(
        scheme.qemu.as_ref().unwrap().path.as_ref().unwrap(),
        &PathBuf::from(tmp_file)
    );

    fs::remove_file(tmp_file).unwrap();
}
