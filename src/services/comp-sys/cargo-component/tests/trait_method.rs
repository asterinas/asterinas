//! This test checks that if cargo-component can control method and trait method

#![feature(once_cell)]

use std::path::PathBuf;
use test_utils::{cargo_clean, cargo_component, clean_after_test};
mod test_utils;

#[test]
fn trait_method() {
    let root_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_dir = root_dir.join("target").join("trait_method_test");
    let cwd = root_dir.join("tests").join("trait_method_test");
    let output = cargo_clean(&cwd, &target_dir);
    assert!(output.status.success());

    let output = cargo_component(&cwd, &target_dir);
    let stderr = String::from_utf8_lossy(&output.stderr);
    println!("stderr: {stderr}");
    
    assert!(output.status.success());
    assert!(stderr.contains("access controlled entry point is disallowed"));
    assert!(stderr.contains("access foo::Foo::method in bar"));
    assert!(stderr.contains("access foo::FooTrait::trait_associate_fn in bar"));
    assert!(stderr.contains("access foo::FooTrait::trait_method in bar"));

    clean_after_test(&cwd);
}
