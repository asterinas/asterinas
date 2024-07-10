// SPDX-License-Identifier: MPL-2.0

use std::{fs, path::PathBuf};

use crate::util::{add_tdx_scheme_to_manifest, cargo_osdk, depends_on_local_ostd};

#[test]
fn create_and_run_kernel() {
    let work_dir = "/tmp";
    let os_name = "myos";

    let os_dir = PathBuf::from(work_dir).join(os_name);

    if os_dir.exists() {
        fs::remove_dir_all(&os_dir).unwrap();
    }

    let mut new_command = cargo_osdk(&["new", "--kernel", os_name]);
    new_command.current_dir(work_dir);
    new_command.ok().unwrap();

    // Makes the kernel depend on local OSTD
    let manifest_path = os_dir.join("Cargo.toml");
    depends_on_local_ostd(manifest_path);
    let tdx_enabled = std::env::var("INTEL_TDX").is_ok();
    if tdx_enabled {
        let osdk_path = os_dir.join("OSDK.toml");
        add_tdx_scheme_to_manifest(&osdk_path).unwrap();
    }
    let mut build_command = if tdx_enabled {
        cargo_osdk(&["build", "--scheme", "tdx"])
    } else {
        cargo_osdk(&["build"])
    };
    build_command.current_dir(&os_dir);
    build_command.ok().unwrap();

    let mut run_command = if tdx_enabled {
        cargo_osdk(&["run", "--scheme", "tdx"])
    } else {
        cargo_osdk(&["run"])
    };
    run_command.current_dir(&os_dir);
    let output = run_command.output().unwrap();
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

    let mut new_command = cargo_osdk(&["new", module_name]);
    new_command.current_dir(work_dir);
    new_command.ok().unwrap();

    let manifest_path = module_dir.join("Cargo.toml");
    depends_on_local_ostd(manifest_path.clone());
    let tdx_enabled = std::env::var("INTEL_TDX").is_ok();
    if tdx_enabled {
        let osdk_path = module_dir.join("OSDK.toml");
        add_tdx_scheme_to_manifest(&osdk_path).unwrap();
    }

    let mut test_command = if tdx_enabled {
        cargo_osdk(&["test", "--scheme", "tdx"])
    } else {
        cargo_osdk(&["test"])
    };
    test_command.current_dir(&module_dir);
    test_command.ok().unwrap();

    fs::remove_dir_all(&module_dir).unwrap();
}
