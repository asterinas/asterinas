// Licensed under the Apache License, Version 2.0 or the MIT License.
// Copyright (C) 2023-2024 Ant Group.

//! This test checks that if Components.toml is missed, the compiler will panic.

#![feature(once_cell)]

mod test_utils;

#[test]
fn missing_toml() {
    let stderr = run_cargo_component_cmd!();
    assert!(stderr.contains("cannot find components.toml"));
}
