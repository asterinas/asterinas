// SPDX-License-Identifier: MPL-2.0

//! The base crate is the OSDK generated crate that is ultimately built by cargo.
//! It will depend on the kernel crate.
//!

use std::path::Path;
use std::process::Command;
use std::str::FromStr;
use std::{fs, process};

use crate::error::Errno;
use crate::error_msg;

pub fn new_base_crate(
    base_crate_path: impl AsRef<Path>,
    dep_crate_name: &str,
    dep_crate_path: impl AsRef<Path>,
) {
    if base_crate_path.as_ref().exists() {
        std::fs::remove_dir_all(&base_crate_path).unwrap();
    }

    let mut cmd = Command::new("cargo");
    cmd.arg("new").arg("--bin").arg(base_crate_path.as_ref());
    cmd.arg("--vcs").arg("none");

    if !cmd.status().unwrap().success() {
        error_msg!(
            "Failed to create base crate at: {:#?}",
            base_crate_path.as_ref()
        );
        process::exit(Errno::CreateBaseCrate as _);
    }

    // Set the current directory to the target osdk directory
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&base_crate_path).unwrap();

    // Add linker.ld file
    let linker_ld = include_str!("x86_64-custom.ld.template");
    fs::write("x86_64-custom.ld", linker_ld).unwrap();

    // Add target json file
    let target_json = include_str!("x86_64-custom.json.template");
    fs::write("x86_64-custom.json", target_json).unwrap();

    // Overrite the main.rs file
    let main_rs = include_str!("main.rs.template");
    // Replace all occurence of `#TARGET_NAME#` with the `dep_crate_name`
    let main_rs = main_rs.replace("#TARGET_NAME#", &dep_crate_name.replace("-", "_"));
    fs::write("src/main.rs", main_rs).unwrap();

    // Add dependencies to the Cargo.toml
    add_manifest_dependency(dep_crate_name, dep_crate_path);

    // Copy the manifest configurations from the target crate to the base crate
    copy_manifest_configurations(base_crate_path);

    // Get back to the original directory
    std::env::set_current_dir(&original_dir).unwrap();
}

fn add_manifest_dependency(crate_name: &str, crate_path: impl AsRef<Path>) {
    let mainfest_path = "Cargo.toml";

    let mut manifest: toml::Table = {
        let content = fs::read_to_string(mainfest_path).unwrap();
        toml::from_str(&content).unwrap()
    };

    let dependencies = manifest.get_mut("dependencies").unwrap();

    let dep = toml::Table::from_str(&format!(
        "{} = {{ path = \"{}\"}}",
        crate_name,
        crate_path.as_ref().display()
    ))
    .unwrap();
    dependencies.as_table_mut().unwrap().extend(dep);

    let content = toml::to_string(&manifest).unwrap();
    fs::write(mainfest_path, content).unwrap();
}

fn copy_manifest_configurations(target_crate_path: impl AsRef<Path>) {
    let target_manifest_path = target_crate_path.as_ref().join("Cargo.toml");
    let manifest_path = "Cargo.toml";

    let target_manifest: toml::Table = {
        let content = fs::read_to_string(target_manifest_path).unwrap();
        toml::from_str(&content).unwrap()
    };

    let mut manifest: toml::Table = {
        let content = fs::read_to_string(manifest_path).unwrap();
        toml::from_str(&content).unwrap()
    };

    // Copy the profile configurations
    let profile = target_manifest.get("profile");
    if let Some(profile) = profile {
        manifest.insert("profile".to_string(), profile.clone());
    }

    let content = toml::to_string(&manifest).unwrap();
    fs::write(manifest_path, content).unwrap();
}
