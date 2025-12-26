#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

. test-framework.sh

start_test "podman"

test_step "Run alpine container"
run_and_expect "podman run --name=c1 docker.io/library/alpine ls /etc" "alpine-release"

test_step "List images"
run_and_expect "podman image ls" "docker.io/library/alpine"

test_step "List containers"
run_and_expect "podman ps -a" "Exited (0)"

test_step "Remove container"
run_command "podman rm c1"

finish_test