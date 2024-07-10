// SPDX-License-Identifier: MPL-2.0

use std::{fs, path::PathBuf};

use crate::util::{cargo_osdk, edit_config_files};

#[test]
fn create_and_run_kernel() {
    let work_dir = "/tmp";
    let os_name = "myos";

    let os_dir = PathBuf::from(work_dir).join(os_name);

    if os_dir.exists() {
        fs::remove_dir_all(&os_dir).unwrap();
    }

    let mut command = cargo_osdk(&["new", "--kernel", os_name]);
    command.current_dir(work_dir);
    command.ok().unwrap();

    // Makes the kernel depend on local OSTD
    edit_config_files(&os_dir);

    let mut command = cargo_osdk(&["build"]);
    command.current_dir(&os_dir);
    command.ok().unwrap();

    let mut command = cargo_osdk(&["run"]);
    command.current_dir(&os_dir);
    let output = command.output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(stdout.contains("Hello world from guest kernel!"));

    fs::remove_dir_all(&os_dir).unwrap();
}

#[test]
fn create_and_test_library() {
    let work_dir = "/tmp";
    let module_name = "mylib";

    let module_dir = PathBuf::from(work_dir).join(module_name);

    if module_dir.exists() {
        fs::remove_dir_all(&module_dir).unwrap();
    }

    let mut command = cargo_osdk(&["new", module_name]);
    command.current_dir(work_dir);
    command.ok().unwrap();

    edit_config_files(&module_dir);

    let mut command = cargo_osdk(&["test"]);
    command.current_dir(&module_dir);
    command.ok().unwrap();

    fs::remove_dir_all(&module_dir).unwrap();
}
