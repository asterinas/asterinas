// SPDX-License-Identifier: MPL-2.0

//! The test suite for containerization and virtualization applications on Asterinas NixOS.
//!
//! See `test/nixos/README.md#documentation-maintenance` for sync requirements
//! between this test suite and the corresponding "Verified Usage" book section.

use nixos_test_framework::*;

nixos_test_main!();

// ============================================================================
// Podman
// ============================================================================

#[nixos_test]
fn podman_run_alpine_container(nixos_shell: &mut Session) -> Result<(), Error> {
    // Run alpine container
    nixos_shell.run_cmd_and_expect(
        "podman run --name=c1 docker.io/library/alpine ls /etc",
        "alpine-release",
    )?;

    // List images
    nixos_shell.run_cmd_and_expect("podman image ls", "docker.io/library/alpine")?;

    // List containers
    nixos_shell.run_cmd_and_expect("podman ps -a", "Exited (0)")?;

    // Remove container
    nixos_shell.run_cmd("podman rm c1")?;

    Ok(())
}

#[nixos_test]
fn podman_open_interactive_session(nixos_shell: &mut Session) -> Result<(), Error> {
    let container_session_desc =
        SessionDesc::new("/ #", "podman run -it docker.io/library/alpine", "exit");

    nixos_shell.enter_session_and_run(container_session_desc, |alpine_shell| {
        alpine_shell.run_cmd_and_expect("ls /etc/alpine-release", "/etc/alpine-release")?;
        Ok(())
    })?;

    Ok(())
}

// ============================================================================
// Skopeo
// ============================================================================

#[nixos_test]
fn skopeo_inspect_image(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(
        "skopeo inspect docker://docker.io/library/alpine:latest",
        "Digest",
    )?;
    nixos_shell.run_cmd_and_expect("skopeo list-tags docker://docker.io/library/alpine", "Tags")?;
    Ok(())
}
