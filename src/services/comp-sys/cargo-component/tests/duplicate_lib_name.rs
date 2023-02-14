//! This test checks that if two components have same name, the compiler will panic.

#![feature(once_cell)]

use std::path::PathBuf;
use test_utils::{cargo_clean, cargo_component};
mod test_utils;

#[test]
fn duplicate_lib_name() {
    let root_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_dir = root_dir.join("target").join("duplicate_lib_name_test");
    let cwd = root_dir.join("tests").join("duplicate_lib_name_test");
    let output = cargo_clean(&cwd, &target_dir);
    assert!(output.status.success());

    let output = cargo_component(&cwd, &target_dir);
    let stderr = String::from_utf8_lossy(&output.stderr);
    println!("stderr: {stderr}");

    assert!(!output.status.success());
    assert!(stderr.contains("duplicate library names"));
}
