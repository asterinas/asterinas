#![allow(unused)]

use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::sync::LazyLock;

pub static CARGO_COMPONENT_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let mut path = std::env::current_exe().unwrap();
    assert!(path.pop()); // deps
    path.set_file_name("cargo-component");
    path
});

pub fn cargo_clean(cwd: &PathBuf, target_dir: &PathBuf) -> Output {
    Command::new("cargo")
        .arg("clean")
        .current_dir(cwd)
        .env("CARGO_TARGET_DIR", target_dir)
        .output()
        .unwrap()
}

pub fn cargo_component(cwd: &PathBuf, target_dir: &PathBuf) -> Output {
    Command::new(&*CARGO_COMPONENT_PATH)
        .current_dir(cwd)
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_TARGET_DIR", target_dir)
        .output()
        .unwrap()
}

pub fn clean_after_test(cwd: &PathBuf) {
    let cargo_lock = cwd.join("Cargo.lock");
    std::fs::remove_file(cargo_lock);
}
