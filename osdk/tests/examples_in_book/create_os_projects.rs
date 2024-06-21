// SPDX-License-Identifier: MPL-2.0

use std::{fs, path::PathBuf};

use crate::util::{cargo_osdk, depends_on_local_ostd};

#[test]
fn create_a_kernel_project() {
    let workdir = "/tmp";
    let kernel = "my_foo_os";

    let kernel_path = PathBuf::from(workdir).join(kernel);

    if kernel_path.exists() {
        fs::remove_dir_all(&kernel_path).unwrap();
    }

    cargo_osdk(&["new", "--kernel", kernel])
        .current_dir(workdir)
        .unwrap();

    assert!(kernel_path.is_dir());
    assert!(kernel_path.join("Cargo.toml").is_file());
    assert!(kernel_path.join("rust-toolchain.toml").is_file());

    depends_on_local_ostd(kernel_path.join("Cargo.toml"));

    fs::remove_dir_all(&kernel_path).unwrap();
}

#[test]
fn create_a_library_project() {
    let workdir = "/tmp";
    let module = "my_foo_module";

    let module_path = PathBuf::from(workdir).join(module);
    if module_path.exists() {
        fs::remove_dir_all(&module_path).unwrap();
    }

    cargo_osdk(&["new", module]).current_dir(workdir).unwrap();

    assert!(module_path.is_dir());
    assert!(module_path.join("Cargo.toml").is_file());
    assert!(module_path.join("rust-toolchain.toml").is_file());

    fs::remove_dir_all(&module_path).unwrap();
}
