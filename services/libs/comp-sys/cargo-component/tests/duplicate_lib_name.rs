// Licensed under the Apache License, Version 2.0 or the MIT License.
// Copyright (C) 2023-2024 Ant Group.

//! This test checks that if two components have same name, the compiler will panic.

#![feature(once_cell)]

mod test_utils;

#[test]
fn duplicate_lib_name() {
    let stderr = run_cargo_component_cmd!();
    assert!(stderr.contains("duplicate library names"));
}
