//! This test checks that if Components.toml is missed, the compiler will panic.

#![feature(once_cell)]

mod test_utils;

#[test]
fn missing_toml() {
    let stderr = run_cargo_component_cmd!();
    assert!(stderr.contains("cannot find components.toml"));
}
