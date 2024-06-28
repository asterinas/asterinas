// Licensed under the Apache License, Version 2.0 or the MIT License.
// Copyright (C) 2023-2024 Ant Group.

//! This test checks that visiting controlled resources in whitelist is allowed.

#![feature(once_cell)]

mod test_utils;

#[test]
fn test() {
    let stderr = run_cargo_component_cmd!();
    assert!(!stderr.contains("access controlled entry point is disallowed"));
}
