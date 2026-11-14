// SPDX-License-Identifier: MPL-2.0

//! Minimal containerd -> Kata -> QEMU TCG smoke test.

use nixos_test_framework::*;

nixos_test_main!();

fn wait_for_containerd(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(
        "i=0; while [ \"$i\" -lt 90 ]; do \
           if [ \"$(systemctl is-active containerd 2>/dev/null)\" = active ] \
              && [ -S /run/containerd/containerd.sock ]; then \
             echo CONTAINERD_READY; break; \
           fi; \
           i=$((i + 1)); sleep 2; \
         done",
        "CONTAINERD_READY",
    )
}

#[nixos_test]
fn kata_tcg_native_snapshotter(nixos_shell: &mut Session) -> Result<(), Error> {
    // Keep long shell commands intact so the PTY harness can match their echo.
    nixos_shell.run_cmd("stty cols 1000")?;
    wait_for_containerd(nixos_shell)?;
    nixos_shell.run_cmd(": > /var/log/containerd.log")?;

    // Avoid `ctr version`: containerd's introspection RPC stats the host
    // `/proc/<pid>/ns/pid`, which this smoke test does not require.
    nixos_shell.run_cmd_and_expect("containerd --version", "containerd")?;
    nixos_shell.run_cmd_and_expect(
        "test -c /dev/vhost-vsock && echo VHOST_VSOCK_READY",
        "VHOST_VSOCK_READY",
    )?;
    nixos_shell.run_cmd_and_expect(
        "grep -q 'internetworking_model = \"none\"' /etc/kata-containers/configuration.toml \
         && grep -q '^disable_new_netns = true' /etc/kata-containers/configuration.toml \
         && grep -q '^disable_vhost_net = true' /etc/kata-containers/configuration.toml \
         && grep -q '^sandbox_cgroup_only = true' /etc/kata-containers/configuration.toml \
         && grep -q -- '--inode-file-handles=never' \
              /etc/kata-containers/configuration.toml \
         && echo KATA_HOST_NETNS_BYPASSED",
        "KATA_HOST_NETNS_BYPASSED",
    )?;

    nixos_shell.run_cmd_and_expect(
        "ctr images import --local --snapshotter native --platform linux/amd64 \
         /etc/kata-tcg/busybox.tar \
         > /tmp/kata-tcg-import.log 2>&1; \
         status=$?; cat /tmp/kata-tcg-import.log; \
         echo KATA_TCG_IMPORT_STATUS_$status",
        "KATA_TCG_IMPORT_STATUS_0",
    )?;
    nixos_shell.run_cmd_and_expect(
        "ctr images list -q | grep -qx docker.io/library/busybox:latest \
         && echo KATA_TCG_IMAGE_READY",
        "KATA_TCG_IMAGE_READY",
    )?;
    nixos_shell.run_cmd_and_expect(
        "test \"$(stat -c %a /run/containerd/s)\" = 700 \
         && echo KATA_TCG_SHIM_DIR_READY",
        "KATA_TCG_SHIM_DIR_READY",
    )?;
    nixos_shell.run_cmd_and_expect(
        "ctr tasks rm -f kata-tcg-smoke >/dev/null 2>&1 || true; \
         ctr containers rm kata-tcg-smoke >/dev/null 2>&1 || true; \
         ctr snapshots --snapshotter native rm kata-tcg-smoke \
           >/dev/null 2>&1 || true; \
         echo KATA_TCG_STALE_STATE_CLEANED",
        "KATA_TCG_STALE_STATE_CLEANED",
    )?;

    nixos_shell.run_cmd(
        "rm -f /tmp/kata-tcg-qemu.argv /tmp/kata-tcg-run.status \
           /tmp/kata-tcg-inner-console.log; \
         (timeout 300s ctr run --rm --snapshotter native \
            --runtime io.containerd.kata.v2 \
            docker.io/library/busybox:latest kata-tcg-smoke \
            /bin/sh -c 'echo KATA_TCG_OK' \
          > /tmp/kata-tcg-run.log 2>&1; \
          echo $? > /tmp/kata-tcg-run.status) & \
         run_pid=$!; \
         i=0; \
         qemu_captured=0; \
         console_reader_started=0; \
         while [ \"$i\" -lt 300 ]; do \
           qemu_pid=$(pgrep -f '[q]emu-system-x86_64' 2>/dev/null | head -n1); \
           if [ -n \"$qemu_pid\" ] && [ \"$qemu_captured\" -eq 0 ]; then \
             tr '\\0' '\\n' < /proc/$qemu_pid/cmdline \
               > /tmp/kata-tcg-qemu.argv; \
             qemu_captured=1; \
           fi; \
           console_socket=/run/vc/vm/kata-tcg-smoke/console.sock; \
           if [ \"$console_reader_started\" -eq 0 ] \
              && [ -S \"$console_socket\" ]; then \
             timeout 280s perl -MSocket=AF_UNIX,SOCK_STREAM,sockaddr_un -e \
               'socket(S,AF_UNIX,SOCK_STREAM,0) or die $!; \
                connect(S,sockaddr_un($ARGV[0])) or die $!; \
                while (sysread(S,$buf,4096)) { print $buf; }' \
               \"$console_socket\" > /tmp/kata-tcg-inner-console.log 2>&1 & \
             console_reader_started=1; \
           fi; \
           if [ \"$i\" -eq 20 ] || [ \"$i\" -eq 60 ]; then \
             echo ===KATA_TCG_PROCESS_SNAPSHOT_$i===; \
             ps -eo pid,ppid,stat,wchan:32,comm,args; \
             echo ===KATA_TCG_CONTAINERD_STATE_$i===; \
             find /run/containerd -maxdepth 6 -ls 2>&1; \
             echo ===KATA_TCG_RUNTIME_STATE_$i===; \
             find /run/kata-containers /run/vc -maxdepth 6 -ls 2>&1; \
             echo ===KATA_TCG_CONTAINERD_LOG_$i===; \
             tail -n 300 /var/log/containerd.log 2>&1; \
             echo ===KATA_TCG_INNER_CONSOLE_$i===; \
             tail -n 300 /tmp/kata-tcg-inner-console.log 2>&1; \
           fi; \
           if ! kill -0 \"$run_pid\" 2>/dev/null; then \
             break; \
           fi; \
           i=$((i + 1)); sleep 1; \
         done; \
         wait \"$run_pid\" || true",
    )?;
    nixos_shell.run_cmd_and_expect(
        "status=$(cat /tmp/kata-tcg-run.status); \
         if [ \"$status\" -ne 0 ]; then \
           cat /tmp/kata-tcg-run.log; \
           test ! -e /tmp/kata-tcg-qemu.argv \
             || { echo ===KATA_TCG_QEMU_ARGV===; cat /tmp/kata-tcg-qemu.argv; }; \
           echo ===KATA_TCG_FINAL_PROCESSES===; \
           ps -eo pid,ppid,stat,wchan:32,comm,args; \
           echo ===KATA_TCG_FINAL_CONTAINERD_LOG===; \
           tail -n 500 /var/log/containerd.log 2>&1; \
           journalctl --no-pager -u containerd.service; \
           echo ===KATA_TCG_FINAL_INNER_CONSOLE===; \
           cat /tmp/kata-tcg-inner-console.log 2>&1; \
         fi; \
         echo KATA_TCG_STATUS_$status",
        "KATA_TCG_STATUS_0",
    )?;
    nixos_shell.run_cmd_and_expect("grep 'accel=tcg' /tmp/kata-tcg-qemu.argv", "accel=tcg")?;
    nixos_shell.run_cmd_and_expect("grep -Fx KATA_TCG_OK /tmp/kata-tcg-run.log", "KATA_TCG_OK")?;
    nixos_shell.run_cmd_and_expect(
        "ctr containers list -q > /tmp/kata-tcg-containers.list \
         && if grep -qx kata-tcg-smoke /tmp/kata-tcg-containers.list; then \
           echo KATA_TCG_LEAK; \
         else \
           echo KATA_TCG_CLEAN; \
         fi",
        "KATA_TCG_CLEAN",
    )?;

    Ok(())
}
