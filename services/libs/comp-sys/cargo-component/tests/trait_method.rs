// Licensed under the Apache License, Version 2.0 or the MIT License.
// Copyright (C) 2023-2024 Ant Group.

//! This test checks that if cargo-component can control method and trait method

#![feature(once_cell)]

mod test_utils;

#[test]
fn trait_method() {
    let stderr = run_cargo_component_cmd!();
    assert!(stderr.contains("access controlled entry point is disallowed"));
    assert!(stderr.contains("access foo::Foo::method in bar"));
    assert!(stderr.contains("access foo::FooTrait::trait_associate_fn in bar"));
    assert!(stderr.contains("access foo::FooTrait::trait_method in bar"));
}
