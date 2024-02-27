// SPDX-License-Identifier: MPL-2.0

//! The base crate is the OSDK generated crate that is ultimately built by cargo.
//! It will depend on the kernel crate.
//!

use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::util::get_cargo_metadata;

pub fn new_base_crate(
    base_crate_path: impl AsRef<Path>,
    dep_crate_name: &str,
    dep_crate_path: impl AsRef<Path>,
) {
    let workspace_root = {
        let meta = get_cargo_metadata(None::<&str>, None::<&[&str]>).unwrap();
        PathBuf::from(meta.get("workspace_root").unwrap().as_str().unwrap())
    };

    if base_crate_path.as_ref().exists() {
        std::fs::remove_dir_all(&base_crate_path).unwrap();
    }

    let dep_crate_version = {
        let cargo_toml = dep_crate_path.as_ref().join("Cargo.toml");
        let cargo_toml = fs::read_to_string(cargo_toml).unwrap();
        let cargo_toml: toml::Value = toml::from_str(&cargo_toml).unwrap();
        let dep_version = cargo_toml
            .get("package")
            .unwrap()
            .as_table()
            .unwrap()
            .get("version")
            .unwrap()
            .as_str()
            .unwrap();
        dep_version.to_string()
    };

    // Create the directory
    fs::create_dir_all(&base_crate_path).unwrap();
    // Create the src directory
    fs::create_dir_all(base_crate_path.as_ref().join("src")).unwrap();

    // Write Cargo.toml
    let cargo_toml = include_str!("Cargo.toml.template");
    let cargo_toml = cargo_toml.replace("#NAME#", &(dep_crate_name.to_string() + "-osdk-bin"));
    let cargo_toml = cargo_toml.replace("#VERSION#", &dep_crate_version);
    fs::write(base_crate_path.as_ref().join("Cargo.toml"), cargo_toml).unwrap();

    // Set the current directory to the target osdk directory
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&base_crate_path).unwrap();

    // Add linker.ld file
    let linker_ld = include_str!("x86_64.ld.template");
    fs::write("x86_64.ld", linker_ld).unwrap();

    // Overrite the main.rs file
    let main_rs = include_str!("main.rs.template");
    // Replace all occurence of `#TARGET_NAME#` with the `dep_crate_name`
    let main_rs = main_rs.replace("#TARGET_NAME#", &dep_crate_name.replace('-', "_"));
    fs::write("src/main.rs", main_rs).unwrap();

    // Add dependencies to the Cargo.toml
    add_manifest_dependency(dep_crate_name, dep_crate_path);

    // Copy the manifest configurations from the target crate to the base crate
    copy_profile_configurations(workspace_root);

    // Get back to the original directory
    std::env::set_current_dir(original_dir).unwrap();
}

fn add_manifest_dependency(crate_name: &str, crate_path: impl AsRef<Path>) {
    let mainfest_path = "Cargo.toml";

    let mut manifest: toml::Table = {
        let content = fs::read_to_string(mainfest_path).unwrap();
        toml::from_str(&content).unwrap()
    };

    // Check if "dependencies" key exists, create it if it doesn't
    if !manifest.contains_key("dependencies") {
        manifest.insert(
            "dependencies".to_string(),
            toml::Value::Table(toml::Table::new()),
        );
    }

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

fn copy_profile_configurations(workspace_root: impl AsRef<Path>) {
    let target_manifest_path = workspace_root.as_ref().join("Cargo.toml");
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
