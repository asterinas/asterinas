// Licensed under the Apache License, Version 2.0 or the MIT License.
// Copyright (C) 2023-2024 Ant Group.

#![allow(unused)]

use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::sync::LazyLock;

#[macro_export]
macro_rules! run_cargo_component_cmd {
    () => ({
        let file = file!();
        let path = std::path::PathBuf::from(file);
        let filename = path.file_name().unwrap().to_string_lossy();
        let test_name = format!("{}_test", filename.trim_end_matches(".rs"));
        test_utils::run_cargo_component(&test_name)
    });
}

static CARGO_COMPONENT_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let mut path = std::env::current_exe().unwrap();
    assert!(path.pop()); // deps
    path.set_file_name("cargo-component");
    path
});

pub fn run_cargo_component(test_name: &str) -> String {
    let root_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_dir = root_dir.join("target").join(test_name);
    let cwd = root_dir.join("tests").join(test_name);
    let output = cargo_clean(&cwd, &target_dir);
    assert!(output.status.success());

    let output = cargo_component(&cwd, &target_dir);
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    println!("stderr: {stderr}");
    clean_after_test(&cwd, &target_dir);
    stderr
}

fn cargo_clean(cwd: &PathBuf, target_dir: &PathBuf) -> Output {
    Command::new("cargo")
        .arg("clean")
        .current_dir(cwd)
        .env("CARGO_TARGET_DIR", target_dir)
        .output()
        .unwrap()
}

fn cargo_component(cwd: &PathBuf, target_dir: &PathBuf) -> Output {
    Command::new(&*CARGO_COMPONENT_PATH)
        .current_dir(cwd)
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_TARGET_DIR", target_dir)
        .output()
        .unwrap()
}

fn clean_after_test(cwd: &PathBuf, target_dir: &PathBuf) {
    cargo_clean(cwd, target_dir);
    let cargo_lock = cwd.join("Cargo.lock");
    std::fs::remove_file(cargo_lock);
}
