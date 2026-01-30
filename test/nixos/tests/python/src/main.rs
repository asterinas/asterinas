// SPDX-License-Identifier: MPL-2.0

//! The test suite for Python regression tests on Asterinas NixOS.
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
fn python(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("curl https://www.python.org/ftp/python/3.12.12/Python-3.12.12.tar.xz -o Python-3.12.12.tar.xz")?;
    nixos_shell.run_cmd("tar xf Python-3.12.12.tar.xz")?;
    nixos_shell.run_cmd("export PYTHONPATH=$PWD/Python-3.12.12/Lib")?;

    let testcases = std::fs::read_to_string("src/testcases.txt")?;
    for line in testcases.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("#") {
            continue;
        }

        let cmd = format!("python -m test -u all {}", line);
        nixos_shell.run_cmd_and_expect(&cmd, "Result: SUCCESS")?;
    }

    Ok(())
}
