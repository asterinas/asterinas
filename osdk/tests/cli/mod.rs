// SPDX-License-Identifier: MPL-2.0

use crate::util::*;

#[test]
fn cli_help_message() {
    let output = cargo_osdk(&["-h"]).output().unwrap();
    assert_success(&output);
    assert_stdout_contains_msg(&output, "cargo osdk [OPTIONS] <COMMAND>");
}

#[test]
fn cli_new_help_message() {
    let output = cargo_osdk(&["new", "-h"]).output().unwrap();
    assert_success(&output);
    assert_stdout_contains_msg(&output, "cargo osdk new [OPTIONS] <name>");
}

#[test]
fn cli_build_help_message() {
    let output = cargo_osdk(&["build", "-h"]).output().unwrap();
    assert_success(&output);
    assert_stdout_contains_msg(&output, "cargo osdk build [OPTIONS]");
}

#[test]
fn cli_run_help_message() {
    let output = cargo_osdk(&["run", "-h"]).output().unwrap();
    assert_success(&output);
    assert_stdout_contains_msg(&output, "cargo osdk run [OPTIONS]");
}

#[test]
fn cli_test_help_message() {
    let output = cargo_osdk(&["test", "-h"]).output().unwrap();
    assert_success(&output);
    assert_stdout_contains_msg(&output, "cargo osdk test [OPTIONS] [TESTNAME]");
}

#[test]
fn cli_check_help_message() {
    let output = cargo_osdk(&["check", "-h"]).output().unwrap();
    assert_success(&output);
    assert_stdout_contains_msg(&output, "cargo osdk check");
}

#[test]
fn cli_clippy_help_message() {
    let output = cargo_osdk(&["clippy", "-h"]).output().unwrap();
    assert_success(&output);
    assert_stdout_contains_msg(&output, "cargo osdk clippy");
}
