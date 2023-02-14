//! This test checks that if controlled resource not in whitelist is visited, cargo-component will report warning message.

#![feature(once_cell)]

use std::path::PathBuf;
use test_utils::{cargo_clean, cargo_component, clean_after_test};
mod test_utils;

#[test]
fn violate_policy() {
    let root_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_dir = root_dir.join("target").join("violate_policy_test");
    let cwd = root_dir.join("tests").join("violate_policy_test");
    let output = cargo_clean(&cwd, &target_dir);
    assert!(output.status.success());

    let output = cargo_component(&cwd, &target_dir);
    let stderr = String::from_utf8_lossy(&output.stderr);
    println!("stderr: {stderr}");

    assert!(output.status.success());
    assert!(stderr.contains("access controlled entry point is disallowed"));
    assert!(stderr.contains("access foo::foo_add in bar"));
    assert!(stderr.contains("access foo::FOO_ITEM in bar"));
    clean_after_test(&cwd);
}
