// SPDX-License-Identifier: MPL-2.0

//! Device-mapper interface tests on Asterinas NixOS.
//!
//! Currently covers control interface version negotiation only.

use nixos_test_framework::*;

nixos_test_main!();

#[nixos_test]
fn dmsetup_version(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell
        .run_cmd_and_expect_regex("dmsetup version", &Regex::new("Driver\\ version:").unwrap())?;
    Ok(())
}
