//! This test checks that if cargo-component can control reexported entry points.

#![feature(once_cell)]

mod test_utils;

#[test]
fn reexport() {
    let stderr = run_cargo_component_cmd!();
    assert!(!stderr.contains("access controlled entry point is disallowed"));
}
