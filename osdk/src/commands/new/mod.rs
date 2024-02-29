// SPDX-License-Identifier: MPL-2.0

use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process,
    str::FromStr,
};

use crate::{
    cli::NewArgs,
    error::Errno,
    error_msg,
    util::{aster_crate_dep, cargo_new_lib, get_cargo_metadata},
};

pub fn execute_new_command(args: &NewArgs) {
    cargo_new_lib(&args.crate_name);
    let cargo_metadata = get_cargo_metadata(Some(&args.crate_name), None::<&[&str]>).unwrap();
    add_manifest_dependencies(&cargo_metadata, &args.crate_name);
    create_osdk_manifest(&cargo_metadata);
    if args.kernel {
        write_kernel_template(&cargo_metadata, &args.crate_name);
    } else {
        write_library_template(&cargo_metadata, &args.crate_name);
    }
    add_rust_toolchain(&cargo_metadata);
}

/// OSDK assumes that the toolchain used by the kernel should be same same as the toolchain
/// specified in the asterinas workspace.
fn aster_rust_toolchain() -> &'static str {
    include_str!("../../../../rust-toolchain.toml")
}

fn add_manifest_dependencies(cargo_metadata: &serde_json::Value, crate_name: &str) {
    let mainfest_path = get_manifest_path(cargo_metadata, crate_name);

    let mut manifest: toml::Table = {
        let content = fs::read_to_string(mainfest_path).unwrap();
        toml::from_str(&content).unwrap()
    };

    let dependencies = manifest.get_mut("dependencies").unwrap();

    let aster_frame_dep = toml::Table::from_str(&aster_crate_dep("aster-frame")).unwrap();
    dependencies.as_table_mut().unwrap().extend(aster_frame_dep);
    let ktest_dep = toml::Table::from_str(&aster_crate_dep("ktest")).unwrap();
    dependencies.as_table_mut().unwrap().extend(ktest_dep);

    // If we created a workspace by `osdk new`, we should exclude the `base` crate from the workspace.
    if get_cargo_metadata::<&Path, &OsStr>(None, None).is_none() {
        let exclude = toml::Table::from_str(r#"exclude = ["target/osdk/base"]"#).unwrap();
        manifest.insert("workspace".to_string(), toml::Value::Table(exclude));
    }

    let content = toml::to_string(&manifest).unwrap();
    fs::write(mainfest_path, content).unwrap();
}

fn create_osdk_manifest(cargo_metadata: &serde_json::Value) {
    let osdk_manifest_path = {
        let workspace_root = get_workspace_root(cargo_metadata);
        PathBuf::from(workspace_root).join("OSDK.toml")
    };

    if osdk_manifest_path.is_file() {
        // If the OSDK.toml of workspace exists, just return.
        return;
    }

    // Create `OSDK.toml` for the workspace
    // FIXME: we need ovmf for grub-efi, the user may not have it.
    // The apt OVMF repo installs to `/usr/share/OVMF`
    fs::write(
        osdk_manifest_path,
        r#"
[boot]
ovmf = "/usr/share/OVMF"
[qemu]
machine = "q35"
args = [
    "--no-reboot",
    "-m 2G",
    "-nographic",
    "-serial chardev:mux",
    "-monitor chardev:mux",
    "-chardev stdio,id=mux,mux=on,signal=off",
    "-display none",
    "-device isa-debug-exit,iobase=0xf4,iosize=0x04",
]
"#,
    )
    .unwrap();
}

/// Write the default content of `src/kernel.rs`, with contents in provided template.
fn write_kernel_template(cargo_metadata: &serde_json::Value, crate_name: &str) {
    let src_path = get_src_path(cargo_metadata, crate_name);
    let contents = include_str!("kernel.template");
    fs::write(src_path, contents).unwrap();
}

/// Write the default content of `src/lib.rs`, with contents in provided template.
fn write_library_template(cargo_metadata: &serde_json::Value, crate_name: &str) {
    let src_path = get_src_path(cargo_metadata, crate_name);
    let contents = include_str!("lib.template");
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

    for target in targets {
        let name = target.get("name").unwrap().as_str().unwrap();
        if name != crate_name {
            continue;
        }

        let src_path = target.get("src_path").unwrap();
        return src_path.as_str().unwrap();
    }

    panic!("the crate name does not match with any target");
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
        toml::Table::from_str(contents).unwrap()
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
