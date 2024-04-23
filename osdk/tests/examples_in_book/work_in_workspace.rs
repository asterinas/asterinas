// SPDX-License-Identifier: MPL-2.0

use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

use crate::util::cargo_osdk;

#[test]
fn work_in_workspace() {
    let workdir = "/tmp";
    let workspace_name = "myworkspace";

    // Create workspace and its manifest
    let workspace_dir = PathBuf::from(workdir).join(workspace_name);
    if workspace_dir.is_dir() {
        fs::remove_dir_all(&workspace_dir).unwrap();
    }

    fs::create_dir_all(&workspace_dir).unwrap();
    env::set_current_dir(&workspace_dir).unwrap();

    let workspace_toml = include_str!("work_in_workspace_templates/Cargo.toml");
    fs::write(workspace_dir.join("Cargo.toml"), workspace_toml).unwrap();

    // Create a kernel project and a library project
    let kernel = "myos";
    let module = "mymodule";
    cargo_osdk(&["new", "--kernel", kernel]).ok().unwrap();
    cargo_osdk(&["new", module]).ok().unwrap();

    // Add a test function to mymodule/src/lib.rs
    let module_src_path = workspace_dir.join(module).join("src").join("lib.rs");
    assert!(module_src_path.is_file());
    let mut module_src_file = OpenOptions::new()
        .append(true)
        .open(&module_src_path)
        .unwrap();
    module_src_file
        .write_all(include_bytes!(
            "work_in_workspace_templates/mymodule/src/lib.rs"
        ))
        .unwrap();
    module_src_file.flush().unwrap();

    // Add dependency to myos/Cargo.toml
    let kernel_manifest_path = workspace_dir.join(kernel).join("Cargo.toml");
    assert!(kernel_manifest_path.is_file());
    let mut kernel_manifest_file = OpenOptions::new()
        .append(true)
        .open(&kernel_manifest_path)
        .unwrap();
    kernel_manifest_file
        .write_all(include_bytes!(
            "work_in_workspace_templates/myos/Cargo.toml"
        ))
        .unwrap();
    kernel_manifest_file.flush().unwrap();

    // Add the content to myos/src/lib.rs
    let kernel_src_path = workspace_dir.join(kernel).join("src").join("lib.rs");
    assert!(kernel_src_path.is_file());
    fs::write(
        &kernel_src_path,
        include_str!("work_in_workspace_templates/myos/src/lib.rs"),
    )
    .unwrap();

    // Run subcommand build & run
    cargo_osdk(&["build"]).ok().unwrap();
    let output = cargo_osdk(&["run"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(stdout.contains("The available memory is"));

    // Run subcommand test
    cargo_osdk(&["test"]).output().unwrap();

    // Remove the directory
    fs::remove_dir_all(&workspace_dir).unwrap();
}
