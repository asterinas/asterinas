// SPDX-License-Identifier: MPL-2.0

use std::{fs, path::PathBuf, process, str::FromStr};

use crate::{
    cli::NewArgs,
    config::manifest::ProjectType,
    error::Errno,
    error_msg,
    util::{cargo_new_lib, get_cargo_metadata, ostd_dep},
};

pub fn execute_new_command(args: &NewArgs) {
    cargo_new_lib(&args.crate_name);
    let cargo_metadata = get_cargo_metadata(Some(&args.crate_name), None::<&[&str]>).unwrap();
    add_manifest_dependencies(&cargo_metadata, &args.crate_name);
    create_osdk_manifest(&cargo_metadata, &args.project_type());
    exclude_osdk_base(&cargo_metadata);
    write_src_template(&cargo_metadata, &args.crate_name, &args.project_type());
    add_rust_toolchain(&cargo_metadata);
}

/// OSDK assumes that the toolchain used by the kernel should be same same as the toolchain
/// specified in the asterinas workspace.
fn aster_rust_toolchain() -> String {
    // Here we can't just include it in the repository root because that can't be
    // read when publishing. Please ensure update both files when updating the toolchain.
    let template = include_str!("./rust-toolchain.toml.template");
    // Delete first two lines of comments.
    template.lines().skip(2).collect::<Vec<_>>().join("\n")
}

fn add_manifest_dependencies(cargo_metadata: &serde_json::Value, crate_name: &str) {
    let manifest_path = get_manifest_path(cargo_metadata, crate_name);

    let mut manifest: toml::Table = {
        let content = fs::read_to_string(manifest_path).unwrap();
        toml::from_str(&content).unwrap()
    };

    let dependencies = manifest.get_mut("dependencies").unwrap();

    let ostd_dep = toml::Table::from_str(&ostd_dep()).unwrap();
    dependencies.as_table_mut().unwrap().extend(ostd_dep);

    let content = toml::to_string(&manifest).unwrap();
    fs::write(manifest_path, content).unwrap();
}

// Add `target/osdk/base` and `target/osdk/test-base` to `exclude` array of the workspace manifest
fn exclude_osdk_base(metadata: &serde_json::Value) {
    let osdk_run_base_path = "target/osdk/base";
    let osdk_test_base_path = "target/osdk/test-base";

    let workspace_manifest_path = {
        let workspace_root = metadata.get("workspace_root").unwrap().as_str().unwrap();
        format!("{}/Cargo.toml", workspace_root)
    };

    let content = fs::read_to_string(&workspace_manifest_path).unwrap();
    let mut manifest_toml: toml::Table = toml::from_str(&content).unwrap();

    if let Some(workspace) = manifest_toml.get_mut("workspace") {
        let workspace = workspace.as_table_mut().unwrap();

        if let Some(exclude) = workspace.get_mut("exclude") {
            let exclude = exclude.as_array_mut().unwrap();
            if exclude.contains(&toml::Value::String(osdk_run_base_path.to_string()))
                || exclude.contains(&toml::Value::String(osdk_test_base_path.to_string()))
            {
                return;
            }

            exclude.push(toml::Value::String(osdk_run_base_path.to_string()));
            exclude.push(toml::Value::String(osdk_test_base_path.to_string()));
        } else {
            let exclude = vec![
                toml::Value::String(osdk_run_base_path.to_string()),
                toml::Value::String(osdk_test_base_path.to_string()),
            ];
            workspace.insert("exclude".to_string(), toml::Value::Array(exclude));
        }
    } else {
        let exclude =
            toml::Table::from_str(r#"exclude = ["target/osdk/base", "target/osdk/test-base"]"#)
                .unwrap();
        manifest_toml.insert("workspace".to_string(), toml::Value::Table(exclude));
    }

    let content = toml::to_string(&manifest_toml).unwrap();
    fs::write(workspace_manifest_path, content).unwrap();
}

fn create_osdk_manifest(cargo_metadata: &serde_json::Value, type_: &ProjectType) {
    let osdk_manifest_path = {
        let workspace_root = get_workspace_root(cargo_metadata);
        PathBuf::from(workspace_root).join("OSDK.toml")
    };

    if osdk_manifest_path.is_file() {
        // If the OSDK.toml of workspace exists, just return.
        return;
    }

    // Create `OSDK.toml` for the workspace
    let contents = match type_ {
        ProjectType::Kernel => {
            include_str!("kernel.OSDK.toml.template")
        }
        ProjectType::Library => {
            include_str!("lib.OSDK.toml.template")
        }
        ProjectType::Module => {
            todo!()
        }
    };

    if is_in_virtual_workspace(cargo_metadata) {
        // If the project is created in a workspace,
        // the project type should be neither a project nor a library.
        // FIXME: This is only a temporary fix to remove the project type,
        // we may decide the actual type in the future.
        let contents = contents.lines().skip(2).collect::<Vec<_>>().join("\n");
        fs::write(osdk_manifest_path, contents).unwrap();
        return;
    }

    fs::write(osdk_manifest_path, contents).unwrap();
}

/// Write the default content of `src/lib.rs`, with contents in provided template.
fn write_src_template(cargo_metadata: &serde_json::Value, crate_name: &str, type_: &ProjectType) {
    let src_path = get_src_path(cargo_metadata, crate_name);
    let contents = match type_ {
        ProjectType::Kernel => {
            include_str!("kernel.template")
        }
        ProjectType::Library => {
            include_str!("lib.template")
        }
        ProjectType::Module => {
            todo!()
        }
    };
    fs::write(src_path, contents).unwrap();
}

/// Add the rust-toolchain.toml file in workspace root
fn add_rust_toolchain(cargo_metadata: &serde_json::Value) {
    let workspace_root = get_workspace_root(cargo_metadata);

    let rust_toolchain_path = PathBuf::from(workspace_root).join("rust-toolchain.toml");
    if let Ok(true) = rust_toolchain_path.try_exists() {
        let toolchain = {
            let content = fs::read_to_string(&rust_toolchain_path).unwrap();
            toml::Table::from_str(&content).unwrap()
        };

        check_rust_toolchain(&toolchain);
        return;
    }

    let contents = aster_rust_toolchain();
    fs::write(rust_toolchain_path, contents).unwrap();
}

fn get_manifest_path<'a>(cargo_metadata: &'a serde_json::Value, crate_name: &str) -> &'a str {
    let metadata = get_package_metadata(cargo_metadata, crate_name);
    metadata.get("manifest_path").unwrap().as_str().unwrap()
}

fn get_src_path<'a>(cargo_metadata: &'a serde_json::Value, crate_name: &str) -> &'a str {
    let metadata = get_package_metadata(cargo_metadata, crate_name);
    let targets = metadata.get("targets").unwrap().as_array().unwrap();
    assert!(
        targets.len() == 1,
        "there must be one and only one target generated"
    );

    let target = &targets[0];
    let src_path = target.get("src_path").unwrap();
    src_path.as_str().unwrap()
}

fn get_workspace_root(cargo_metadata: &serde_json::Value) -> &str {
    let workspace_root = cargo_metadata.get("workspace_root").unwrap();
    workspace_root.as_str().unwrap()
}

fn get_package_metadata<'a>(
    cargo_metadata: &'a serde_json::Value,
    crate_name: &str,
) -> &'a serde_json::Value {
    let packages = cargo_metadata.get("packages").unwrap();

    let package_metadatas = packages.as_array().unwrap();

    for package_metadata in package_metadatas {
        let name = package_metadata.get("name").unwrap().as_str().unwrap();
        if crate_name == name {
            return package_metadata;
        }
    }

    panic!("cannot find metadata of the crate")
}

fn check_rust_toolchain(toolchain: &toml::Table) {
    let expected = {
        let contents = aster_rust_toolchain();
        toml::Table::from_str(&contents).unwrap()
    };

    let expected = expected.get("toolchain").unwrap().as_table().unwrap();
    let toolchain = toolchain.get("toolchain").unwrap().as_table().unwrap();

    let channel = toolchain.get("channel").unwrap().as_str().unwrap();
    let expected_channel = expected.get("channel").unwrap().as_str().unwrap();

    if channel != expected_channel {
        error_msg!("The current version of rust-toolchain.toml is not compatible with the osdk");
        process::exit(Errno::AddRustToolchain as _);
    }

    let components = toolchain.get("components").unwrap().as_array().unwrap();
    let expected_components = toolchain.get("components").unwrap().as_array().unwrap();

    for expected_component in expected_components {
        if !components.contains(expected_component) {
            error_msg!(
                "rust-toolchain.toml does not contains {}",
                expected_component.as_str().unwrap()
            );
            process::exit(Errno::AddRustToolchain as _);
        }
    }
}

fn is_in_virtual_workspace(cargo_metadata: &serde_json::Value) -> bool {
    let cargo_manifeset_path = {
        let workspace_root = get_workspace_root(cargo_metadata);
        PathBuf::from(workspace_root).join("Cargo.toml")
    };

    let cargo_manifest = {
        let content = fs::read_to_string(cargo_manifeset_path).unwrap();
        toml::Table::from_str(&content).unwrap()
    };

    !cargo_manifest.contains_key("package")
}
