// SPDX-License-Identifier: MPL-2.0

use std::{fs, path::PathBuf, process::Command};

use assert_cmd::output::OutputOkExt;

use crate::util::{cargo_osdk, edit_config_files};

#[test]
fn write_a_kernel_in_100_lines() {
    let workdir = "/tmp";
    let os_name = "kernel_in_100_lines";

    let os_dir = PathBuf::from(workdir).join(os_name);

    if os_dir.exists() {
        fs::remove_dir_all(&os_dir).unwrap()
    }

    // Creates a new kernel project
    cargo_osdk(&["new", "--kernel", os_name])
        .current_dir(&workdir)
        .ok()
        .unwrap();

    edit_config_files(&os_dir);

    // Copies the kernel content
    let kernel_contents = include_str!("write_a_kernel_in_100_lines_templates/lib.rs");
    fs::write(os_dir.join("src").join("lib.rs"), kernel_contents).unwrap();

    // Copies and compiles the user program
    let user_program_contents = include_str!("write_a_kernel_in_100_lines_templates/hello.S");
    fs::write(os_dir.join("hello.S"), user_program_contents).unwrap();
    Command::new("gcc")
        .args(&["-static", "-nostdlib", "hello.S", "-o", "hello"])
        .current_dir(&os_dir)
        .ok()
        .unwrap();

    // Adds align ext as the dependency
    let file_contents = fs::read_to_string(os_dir.join("Cargo.toml")).unwrap();
    let mut manifest: toml::Table = toml::from_str(&file_contents).unwrap();
    let dependencies = manifest
        .get_mut("dependencies")
        .unwrap()
        .as_table_mut()
        .unwrap();
    dependencies.insert(
        "align_ext".to_string(),
        toml::Value::String("0.1.0".to_string()),
    );

    let new_file_content = manifest.to_string();
    fs::write(os_dir.join("Cargo.toml"), new_file_content).unwrap();

    // Runs the kernel
    let output = cargo_osdk(&["run"]).current_dir(&os_dir).ok().unwrap();
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    println!("stdout = {}", stdout);

    fs::remove_dir_all(&os_dir).unwrap();
}
