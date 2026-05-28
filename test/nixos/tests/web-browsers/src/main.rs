// SPDX-License-Identifier: MPL-2.0

//! The test suite for web browsers applications on Asterinas NixOS.
//!
//! See `test/nixos/README.md#documentation-maintenance` for sync requirements
//! between this test suite and the corresponding "Verified Usage" book section.

use nixos_test_framework::*;

nixos_test_main!();

// ============================================================================
// Browsers
// ============================================================================

#[nixos_test]
fn firefox_screenshot_website(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("firefox --headless --screenshot https://example.com")?;
    nixos_shell.run_cmd_and_expect("ls -al screenshot.png", "screenshot.png")?;
    Ok(())
}

#[nixos_test]
fn links2_dump_website(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("links -dump http://example.com", "Example Domain")?;
    Ok(())
}

#[nixos_test]
fn w3m_dump_website(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("w3m -dump http://example.com", "Example Domain")?;
    Ok(())
}
