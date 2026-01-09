// SPDX-License-Identifier: MPL-2.0

//! The test suite for podman on Asterinas NixOS.
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
fn alpine_container_basic(nixos_shell: &mut Session) -> Result<(), Error> {
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
fn alpine_interactive_session(nixos_shell: &mut Session) -> Result<(), Error> {
    let container_session_desc = SessionDesc::new()
        .expect_prompt("/ #")
        .cmd_to_enter("podman run -it docker.io/library/alpine")
        .cmd_to_exit("exit");

    nixos_shell.enter_session_and_run(container_session_desc, |alpine_shell| {
        alpine_shell.run_cmd_and_expect("ls /etc/alpine-release", "/etc/alpine-release")?;
        Ok(())
    })?;

    Ok(())
}
