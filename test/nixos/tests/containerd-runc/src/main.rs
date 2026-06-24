// SPDX-License-Identifier: MPL-2.0

//! The test suite for containerd and runc bring-up on Asterinas NixOS.

use nixos_test_framework::*;

nixos_test_main!();

#[nixos_test]
fn containerd_service_basic(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("systemctl is-active containerd", "active")?;
    nixos_shell.run_cmd_and_expect(
        "test -S /run/containerd/containerd.sock && echo ready",
        "ready",
    )?;
    nixos_shell.run_cmd_and_expect("ctr version", "Server:")?;
    nixos_shell.run_cmd_and_expect("command -v runc", "/run/current-system/sw/bin/runc")?;
    nixos_shell.run_cmd_and_expect(
        concat!(
            "for ns in cgroup ipc mnt net pid user uts; do ",
            "test -L /proc/$(pidof containerd)/ns/$ns || exit 1; ",
            "done; echo ns-ready",
        ),
        "ns-ready",
    )?;

    nixos_shell.run_cmd(
        "tr '\\0' ' ' < /proc/$(pidof containerd)/cmdline > /tmp/containerd-cmdline.txt",
    )?;
    nixos_shell.run_cmd_and_expect(
        "cat /tmp/containerd-cmdline.txt",
        "containerd-config-checked.toml",
    )?;

    Ok(())
}

#[nixos_test]
fn containerd_default_runtime_is_runc(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(
        "CONFIG=$(tr '\\0' '\\n' < /proc/$(pidof containerd)/cmdline | grep 'containerd-config-checked.toml'); grep -n 'io.containerd.runc.v2' \"$CONFIG\" > /tmp/runc-runtime-config.txt",
    )?;
    nixos_shell.run_cmd_and_expect("cat /tmp/runc-runtime-config.txt", "io.containerd.runc.v2")?;

    Ok(())
}

#[nixos_test]
fn cgroup_memory_files_are_readable(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("rm -rf /sys/fs/cgroup/asterinas-memory-probe")?;
    nixos_shell.run_cmd("mkdir /sys/fs/cgroup/asterinas-memory-probe")?;
    nixos_shell.run_cmd_and_expect(
        "cat /sys/fs/cgroup/asterinas-memory-probe/memory.events",
        "oom_kill 0",
    )?;
    nixos_shell.run_cmd_and_expect(
        "cat /sys/fs/cgroup/asterinas-memory-probe/memory.stat",
        "anon 0",
    )?;
    nixos_shell.run_cmd_and_expect(
        "cat /sys/fs/cgroup/asterinas-memory-probe/memory.max",
        "max",
    )?;
    nixos_shell.run_cmd("rmdir /sys/fs/cgroup/asterinas-memory-probe")?;

    Ok(())
}
