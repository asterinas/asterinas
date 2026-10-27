// SPDX-License-Identifier: MPL-2.0

//! Real dmsetup coverage for tableless Device Mapper management on Asterinas NixOS.
//!
//! Guest cases own assertions and cleanup; the official framework owns the VM.
//! No backing disks, mapping targets, or runtime block-device nodes are required.

use nixos_test_framework::*;

nixos_test_main!();

/// Runs a case from the image's script and verifies both status and case marker.
fn run_case(shell: &mut Session, name: &str) -> Result<(), Error> {
    shell.run_cmd_and_expect_regex(
        &format!("sh /etc/dm-management-cases.sh {name}"),
        &Regex::new(&format!("(?m)^CASE_PASS_{name}\\r?$")).unwrap(),
    )
}

#[nixos_test]
fn dmsetup_version_and_major(shell: &mut Session) -> Result<(), Error> {
    run_case(shell, "dmsetup_version_and_major")
}

#[nixos_test]
fn dmsetup_tableless_lifecycle(shell: &mut Session) -> Result<(), Error> {
    run_case(shell, "dmsetup_tableless_lifecycle")
}

#[nixos_test]
fn dmsetup_identity_conflicts_and_minor_reuse(shell: &mut Session) -> Result<(), Error> {
    run_case(shell, "dmsetup_identity_conflicts_and_minor_reuse")
}
