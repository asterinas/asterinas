//! This test checks that visiting controlled resources in whitelist is allowed.

#![feature(once_cell)]

mod test_utils;

#[test]
fn regression() {
    let stderr = run_cargo_component_cmd!();
    assert!(!stderr.contains("access controlled entry point is disallowed"));
}
