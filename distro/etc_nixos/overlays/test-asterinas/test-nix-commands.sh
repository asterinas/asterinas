#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

. test-framework.sh

start_test "nix-commands"

test_step "Test nix-env install and run"
run_command "nix-env -iA nixos.hello"
run_and_expect "hello" "Hello, world!"

test_step "Test nix-env uninstall"
run_command "nix-env -e hello"

test_step "Test nix-shell"
run_and_expect "nix-shell -p hello --command hello" "Hello, world!"

test_step "Test nix-build and run"
run_command "nix-build '<nixpkgs>' -A hello"
run_and_expect "./result/bin/hello" "Hello, world!"

test_step "Test nixos-rebuild"
run_command "echo '{ pkgs, ... }: { environment.systemPackages = [ pkgs.hello ]; }' > /tmp/add-hello.nix"
run_command "echo '{ imports = [ /etc/nixos/configuration.nix /tmp/add-hello.nix ]; }' > /tmp/test-config.nix"
run_command "nixos-rebuild test -I nixos-config=/tmp/test-config.nix"
run_command "rm /tmp/*"

test_step "Run hello after rebuild"
run_command "hash -r"
run_and_expect "hello" "Hello, world!"

finish_test