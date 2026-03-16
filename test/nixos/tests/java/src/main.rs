// SPDX-License-Identifier: MPL-2.0

//! The test suite for JDK tests on Asterinas NixOS.
//!
//! # Document maintenance
//!
//! An application's test suite and its "Verified Usage" section in Asterinas Book
//! should always be kept in sync.
//! So whenever you modify the test suite,
//! review the documentation and see if should be updated accordingly.

use nixos_test_framework::*;

nixos_test_main!();

#[nixos_test]
fn java(nixos_shell: &mut Session) -> Result<(), Error> {
    let testcases = std::fs::read_to_string("src/testcases.txt")?;
    for line in testcases.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("#") {
            continue;
        }

        let cmd = format!("jtreg -v1 -nr -retain:none /tmp/jdk/test/jdk/{}", line);
        nixos_shell.run_cmd_and_expect(&cmd, r"(?m)^Test results: passed: [0-9]+$")?;
    }

    Ok(())
}
