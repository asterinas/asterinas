// SPDX-License-Identifier: MPL-2.0

//! The test suite for <TargetAppName> on Asterinas NixOS.
//!
//! See `test/nixos/README.md#documentation-maintenance` for sync requirements
//! between this test suite and the corresponding "Verified Usage" book section.

use nixos_test_framework::*;

nixos_test_main!();

#[nixos_test]
fn echo_write_file(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'Hello, World!' > out.txt")?;
    nixos_shell.run_cmd_and_expect("ls out.txt", "out.txt")?;

    Ok(())
}
