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

// ============================================================================
// Qemu
// ============================================================================

#[nixos_test]
fn qemu_display_version(nixos_shell: &mut Session) -> Result<(), Error> {
    // Verifies that the QEMU emulator is available and reports its version.
    nixos_shell.run_cmd_and_expect("qemu-system-$(uname -m) --version", "QEMU emulator version")?;
    Ok(())
}

#[nixos_test]
fn qemu_tcg_run_linux(nixos_shell: &mut Session) -> Result<(), Error> {
    // Run a Linux kernel inside QEMU using the TCG software emulator.
    //
    // Use `grep -v /bin/echo` to filter out lines that affect matching, such as:
    //   [    0.000000] Kernel command line: initrd=initrd console=ttyS0 panic=-1 rdinit=/bin/echo Entered userspace
    //   [    0.012005] Command line: initrd=initrd console=ttyS0 panic=-1 rdinit=/bin/echo Entered userspace
    const CMD: &str = concat!(
        "qemu-system-$(uname -m) ",
        "-accel tcg ",
        "-kernel $LINUX_BZIMAGE ",
        "-initrd /run/current-system/initrd ",
        "-nographic -no-reboot ",
        "-append 'console=ttyS0 panic=-1 rdinit=/bin/echo Entered userspace' ",
        "| grep -v /bin/echo"
    );
    nixos_shell.run_cmd_and_expect(CMD, "Entered userspace")?;
    Ok(())
}

#[nixos_test]
fn qemu_tcg_run_aster(nixos_shell: &mut Session) -> Result<(), Error> {
    // Run a Asterinas kernel inside QEMU using the TCG software emulator.
    //
    // Use `grep -v /bin/echo` to filter out lines that affect matching, such as:
    //   [EFI stub] Loaded the cmdline: "console=ttyS0 panic=-1 init=/bin/echo Entered userspace initrd=initrd"
    const CMD: &str = concat!(
        "qemu-system-$(uname -m) ",
        "-accel tcg ",
        "-cpu Icelake-Server ",
        "-machine q35 -m 2G ",
        "-bios $OVMF_PATH ",
        "-kernel /run/current-system/kernel ",
        "-initrd /run/current-system/initrd ",
        "-device isa-debug-exit,iobase=0xf4,iosize=0x04 ",
        "-nographic -no-reboot ",
        "-append 'console=ttyS0 panic=-1 init=/bin/echo Entered userspace' ",
        "| grep -v /bin/echo"
    );
    nixos_shell.run_cmd_and_expect(CMD, "Entered userspace")?;
    Ok(())
}
