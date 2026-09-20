// SPDX-License-Identifier: MPL-2.0

use std::fs::{self, File};

use super::*;

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
    assert_eq!(
        scheme.qemu.as_ref().unwrap().log_file.as_deref(),
        Some(Path::new("qemu-serial.log"))
    );

    // Managed daemon scheme
    let daemon = &scheme.qemu.as_ref().unwrap().with_daemons.as_ref().unwrap()[0];
    assert_eq!(daemon.path, Path::new("test-service"));
    assert_eq!(daemon.args, ["--test"]);

    // Iommu
    let mut scheme = toml_manifest.get_scheme(Some("iommu".to_owned())).clone();
    scheme.inherit(&toml_manifest.default_scheme);
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
    assert_eq!(
        scheme.qemu.as_ref().unwrap().log_file.as_deref(),
        Some(Path::new("qemu-serial.log"))
    );
    let daemon = &scheme.qemu.as_ref().unwrap().with_daemons.as_ref().unwrap()[0];
    assert_eq!(daemon.path, Path::new("test-service"));
    assert_eq!(daemon.args, ["--test"]);

    // Tdx
    let scheme = toml_manifest.get_scheme(Some("tdx".to_owned()));
    assert_eq!(
        scheme.qemu.as_ref().unwrap().path.as_ref().unwrap(),
        &PathBuf::from(tmp_file)
    );

    fs::remove_file(tmp_file).unwrap();
}

#[test]
fn parse_quoted_daemon_arguments() {
    let command = "/path/to/service --name 'hello world' --mode test";
    let parts = shlex::split(command).unwrap();

    assert_eq!(
        parts,
        [
            "/path/to/service",
            "--name",
            "hello world",
            "--mode",
            "test",
        ]
    );
}
