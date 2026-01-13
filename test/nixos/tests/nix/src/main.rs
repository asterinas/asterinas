// SPDX-License-Identifier: MPL-2.0

//! The test suite for nix on Asterinas NixOS.
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
fn nix_env_install(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("nix-env -iA nixos.hello")?;
    nixos_shell.run_cmd_and_expect("hello", "Hello, world!")?;
    nixos_shell.run_cmd("nix-env -e hello")?;
    Ok(())
}

#[nixos_test]
fn nix_shell(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("nix-shell -p hello --command hello", "Hello, world!")?;
    Ok(())
}

#[nixos_test]
fn nix_build(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("nix-build '<nixpkgs>' -A hello")?;
    nixos_shell.run_cmd_and_expect("./result/bin/hello", "Hello, world!")?;
    Ok(())
}

#[nixos_test]
fn nixos_rebuild(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo '{ pkgs, ... }: { environment.systemPackages = [ pkgs.hello ]; }' > /tmp/add-hello.nix")?;
    nixos_shell.run_cmd("echo '{ imports = [ /etc/nixos/configuration.nix /tmp/add-hello.nix ]; }' > /tmp/test-config.nix")?;
    nixos_shell.run_cmd("nixos-rebuild test -I nixos-config=/tmp/test-config.nix")?;
    nixos_shell.run_cmd("rm /tmp/*")?;
    nixos_shell.run_cmd("hash -r")?;
    nixos_shell.run_cmd_and_expect("hello", "Hello, world!")?;
    Ok(())
}
