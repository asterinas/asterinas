// SPDX-License-Identifier: MPL-2.0

//! The test suite for <TargetAppName> on Asterinas NixOS.
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
fn hello_world(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'Hello, World!' > out.txt")?;
    nixos_shell.run_cmd_and_expect("ls out.txt", "out.txt")?;
    
    Ok(())
}