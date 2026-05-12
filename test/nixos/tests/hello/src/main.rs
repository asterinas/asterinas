// SPDX-License-Identifier: MPL-2.0

//! The test suite for hello-asterinas on Asterinas NixOS.

use nixos_test_framework::*;

nixos_test_main!();

#[nixos_test]
fn hello(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("hello-asterinas", "Hello Asterinas!")?;
    Ok(())
}
