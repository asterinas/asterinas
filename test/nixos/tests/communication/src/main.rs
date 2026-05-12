// SPDX-License-Identifier: MPL-2.0

//! The test suite for communication applications on Asterinas NixOS.
//!
//! See `test/nixos/README.md#documentation-maintenance` for sync requirements
//! between this test suite and the corresponding "Verified Usage" book section.

use nixos_test_framework::*;

nixos_test_main!();

// ============================================================================
// Terminal Chat Clients - Irssi
// ============================================================================

#[nixos_test]
fn irssi_connect(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("expect /tmp/irssi.exp", "PASS")?;
    Ok(())
}

// ============================================================================
// Terminal Chat Clients - WeeChat
// ============================================================================

#[nixos_test]
fn weechat_connect(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("expect /tmp/weechat.exp", "PASS")?;
    Ok(())
}
