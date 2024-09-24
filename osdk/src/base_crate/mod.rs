// SPDX-License-Identifier: MPL-2.0

//! The base crate is the OSDK generated crate that is ultimately built by cargo.
//! It will depend on the to-be-built kernel crate or the to-be-tested crate.

use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::util::get_cargo_metadata;

/// Create a new base crate that will be built by cargo.
///
/// The dependencies of the base crate will be the target crate. If
/// `link_unit_test_runner` is set to true, the base crate will also depend on
/// the `ostd-test-runner` crate.
pub fn new_base_crate(
    base_crate_path: impl AsRef<Path>,
    dep_crate_name: &str,
    dep_crate_path: impl AsRef<Path>,
    link_unit_test_runner: bool,
) {
    let workspace_root = {
        let meta = get_cargo_metadata(None::<&str>, None::<&[&str]>).unwrap();
        PathBuf::from(meta.get("workspace_root").unwrap().as_str().unwrap())
    };

    if base_crate_path.as_ref().exists() {
        std::fs::remove_dir_all(&base_crate_path).unwrap();
    }

    let (dep_crate_version, dep_crate_features) = {
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
            .unwrap()
            .to_string();
        let dep_features = cargo_toml
            .get("features")
            .map(|f| f.as_table().unwrap().clone())
            .unwrap_or_default();
        (dep_version, dep_features)
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

    // Add linker script files
    macro_rules! include_linker_script {
        ([$($linker_script:literal),+]) => {$(
            fs::write(
                base_crate_path.as_ref().join($linker_script),
                include_str!(concat!($linker_script, ".template"))
            ).unwrap();
        )+};
    }
    // TODO: currently just x86_64 works; add support for other architectures
    // here when OSTD is ready
    include_linker_script!(["x86_64.ld", "riscv64.ld"]);

    // Overwrite the main.rs file
    let main_rs = include_str!("main.rs.template");
    // Replace all occurrence of `#TARGET_NAME#` with the `dep_crate_name`
    let main_rs = main_rs.replace("#TARGET_NAME#", &dep_crate_name.replace('-', "_"));
    fs::write("src/main.rs", main_rs).unwrap();

    // Add dependencies to the Cargo.toml
    add_manifest_dependency(dep_crate_name, dep_crate_path, link_unit_test_runner);

    // Copy the manifest configurations from the target crate to the base crate
    copy_profile_configurations(workspace_root);

    // Generate the features by copying the features from the target crate
    add_feature_entries(dep_crate_name, &dep_crate_features);

    // Get back to the original directory
    std::env::set_current_dir(original_dir).unwrap();
}

fn add_manifest_dependency(
    crate_name: &str,
    crate_path: impl AsRef<Path>,
    link_unit_test_runner: bool,
) {
    let manifest_path = "Cargo.toml";

    let mut manifest: toml::Table = {
        let content = fs::read_to_string(manifest_path).unwrap();
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

    let target_dep = toml::Table::from_str(&format!(
        "{} = {{ path = \"{}\", default-features = false }}",
        crate_name,
        crate_path.as_ref().display()
    ))
    .unwrap();
    dependencies.as_table_mut().unwrap().extend(target_dep);

    if link_unit_test_runner {
        let dep_str = match option_env!("OSDK_LOCAL_DEV") {
            Some("1") => {
                let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
                let test_kernel_dir = crate_dir.join("test-kernel");
                format!(
                    "osdk-test-kernel = {{ path = \"{}\" }}",
                    test_kernel_dir.display()
                )
            }
            _ => concat!(
                "osdk-test-kernel = { version = \"",
                env!("CARGO_PKG_VERSION"),
                "\" }"
            )
            .to_owned(),
        };
        let test_runner_dep = toml::Table::from_str(&dep_str).unwrap();
        dependencies.as_table_mut().unwrap().extend(test_runner_dep);
    }

    let content = toml::to_string(&manifest).unwrap();
    fs::write(manifest_path, content).unwrap();
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
        manifest.insert(
            "profile".to_string(),
            toml::Value::Table(profile.as_table().unwrap().clone()),
        );
    }

    let content = toml::to_string(&manifest).unwrap();
    fs::write(manifest_path, content).unwrap();
}

fn add_feature_entries(dep_crate_name: &str, features: &toml::Table) {
    let manifest_path = "Cargo.toml";
    let mut manifest: toml::Table = {
        let content = fs::read_to_string(manifest_path).unwrap();
        toml::from_str(&content).unwrap()
    };

    let mut table = toml::Table::new();
    for (feature, value) in features.iter() {
        let value = if feature != &"default".to_string() {
            vec![toml::Value::String(format!(
                "{}/{}",
                dep_crate_name, feature
            ))]
        } else {
            value.as_array().unwrap().clone()
        };
        table.insert(feature.clone(), toml::Value::Array(value));
    }

    manifest.insert("features".to_string(), toml::Value::Table(table));

    let content = toml::to_string(&manifest).unwrap();
    fs::write(manifest_path, content).unwrap();
}
