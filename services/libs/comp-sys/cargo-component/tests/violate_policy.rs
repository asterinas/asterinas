// Licensed under the Apache License, Version 2.0 or the MIT License.
// Copyright (C) 2023-2024 Ant Group.

//! This test checks that if controlled resource not in whitelist is visited, cargo-component will 
//! report warning message.

#![feature(once_cell)]

mod test_utils;

#[test]
fn violate_policy() {
    let stderr = run_cargo_component_cmd!();
    assert!(stderr.contains("access controlled entry point is disallowed"));
    assert!(stderr.contains("access foo3::foo_add in bar3"));
    assert!(stderr.contains("access foo3::FOO_ITEM in bar3"));
}
