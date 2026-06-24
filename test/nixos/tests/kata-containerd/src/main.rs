// SPDX-License-Identifier: MPL-2.0

//! The test suite for containerd and Kata runtime bring-up on Asterinas NixOS.

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
fn kata_runtime_registered(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(
        "command -v containerd-shim-kata-v2",
        "/run/current-system/sw/bin/containerd-shim-kata-v2",
    )?;
    nixos_shell.run_cmd_and_expect(
        "containerd-shim-kata-v2 --version",
        "containerd shim (Rust)",
    )?;
    nixos_shell.run_cmd_and_expect(
        "test -r /etc/kata-containers/configuration.toml && echo kata-rs-config-ready",
        "kata-rs-config-ready",
    )?;
    nixos_shell.run_cmd_and_expect("command -v kata-debug-run-all", "kata-debug-run-all")?;
    nixos_shell.run_cmd_and_expect(
        "test -r /etc/kata-debug/guest/10-run-all.sh && echo kata-debug-ready",
        "kata-debug-ready",
    )?;

    nixos_shell.run_cmd(
        "CONFIG=$(tr '\\0' '\\n' < /proc/$(pidof containerd)/cmdline | grep 'containerd-config-checked.toml'); grep -n 'io.containerd.kata.v2' \"$CONFIG\" > /tmp/kata-runtime-config.txt",
    )?;
    nixos_shell.run_cmd_and_expect("cat /tmp/kata-runtime-config.txt", "io.containerd.kata.v2")?;

    Ok(())
}

/// Reset rootfs-probe state and patch
/// `/etc/kata-debug/guest/02-run-rootfs-probe.sh` with the given container
/// shell command (substituted for the upstream `echo ok-from-kata` command).
///
/// On the first call, the upstream script is backed up to `.orig`; subsequent
/// calls restore from that backup before patching, so multiple test cases can
/// run sequentially without worrying about prior sed state.
///
/// `container_cmd` is interpolated into a sed `s|...|...|` replacement. It
/// must contain neither single quotes nor the `|` character — both would
/// terminate the replacement string. Use shell-safe constructs only.
fn prepare_rootfs_probe(nixos_shell: &mut Session, container_cmd: &str) -> Result<(), Error> {
    assert!(
        !container_cmd.contains('\'') && !container_cmd.contains('|'),
        "container_cmd must not contain ' or |: {container_cmd:?}"
    );

    nixos_shell.run_cmd(
        "rm -rf /tmp/kata-debug-out /tmp/kata-debug-run-all.stdout /tmp/kata-run.txt \
         /tmp/kata-rootfs /tmp/containerd-debug.sock /tmp/containerd-root /tmp/containerd-state",
    )?;

    // Snapshot the upstream script on first run, then restore from snapshot
    // on every call so each test patches from a known-clean baseline.
    nixos_shell.run_cmd(
        "if [ ! -f /etc/kata-debug/guest/02-run-rootfs-probe.sh.orig ]; then \
           cp /etc/kata-debug/guest/02-run-rootfs-probe.sh \
              /etc/kata-debug/guest/02-run-rootfs-probe.sh.orig; \
         fi; \
         cp /etc/kata-debug/guest/02-run-rootfs-probe.sh.orig \
            /etc/kata-debug/guest/02-run-rootfs-probe.sh",
    )?;

    // Normalize bare `sh -c '...'` to `/bin/sh -c '...'` (upstream form).
    nixos_shell.run_cmd(
        "sed -i \"s|  sh -c 'echo ok-from-kata'|  /bin/sh -c 'echo ok-from-kata'|\" \
         /etc/kata-debug/guest/02-run-rootfs-probe.sh",
    )?;

    // Replace the upstream `/bin/sh -c 'echo ok-from-kata'` with our command.
    nixos_shell.run_cmd(&format!(
        "sed -i \"s|/bin/sh -c 'echo ok-from-kata'|/bin/sh -c '{container_cmd}'|\" \
         /etc/kata-debug/guest/02-run-rootfs-probe.sh"
    ))?;

    // Copy the bash binary into rootfs (the upstream `ln -sf` symlink doesn't
    // resolve inside the container's mount view).
    nixos_shell.run_cmd(
        "sed -i 's|ln -sf \"$SH\" \"$ROOTFS/bin/sh\"|cp \"$SH\" \"$ROOTFS/bin/sh\"|' \
         /etc/kata-debug/guest/02-run-rootfs-probe.sh",
    )?;
    // `--rm` deletes the container before we can read its task state.
    nixos_shell.run_cmd(
        "sed -i 's|run --rootfs --rm|run --rootfs|' /etc/kata-debug/guest/02-run-rootfs-probe.sh",
    )?;
    nixos_shell.run_cmd(
        "sed -i 's|timeout 120s ctr|timeout 80s ctr|' /etc/kata-debug/guest/02-run-rootfs-probe.sh",
    )?;

    // PT_INTERP fallback: the rootfs ldd-copy puts the dynamic loader at
    // `lib64/`, but NixOS bash embeds an absolute `/nix/store/<glibc>/lib/...`
    // PT_INTERP path. Mirror `ld-linux*` between `lib/` and `lib64/` inside
    // each glibc store dir before the container starts.
    //
    // Skipped if the source script (`guest-scripts/02-run-rootfs-probe.sh`)
    // already includes the same fix (idempotent via grep).
    nixos_shell.run_cmd(
        r##"if ! grep -q PT_INTERP_DETECTED /etc/kata-debug/guest/02-run-rootfs-probe.sh; then cat > /tmp/kata-ptinterp-block <<'EOF'
# PT_INTERP detection (no readelf on minimal NixOS) + lib/lib64 mirror.
INTERP=$(LC_ALL=C tr '\0' '\n' < "$SH" 2>/dev/null | grep -m1 '/ld-linux' || true)
echo "PT_INTERP_DETECTED=$INTERP" | tee -a "$OUT_DIR/probe.log"
if [ -n "$INTERP" ]; then
  mkdir -p "$ROOTFS$(dirname "$INTERP")"
  if [ -e "$INTERP" ] && [ ! -e "$ROOTFS$INTERP" ]; then
    cp "$INTERP" "$ROOTFS$INTERP"
  fi
fi
for d in "$ROOTFS"/nix/store/*-glibc-*/; do
  [ -d "$d" ] || continue
  mkdir -p "$d"lib "$d"lib64
  for f in "$d"lib64/ld-linux* "$d"lib/ld-linux*; do
    [ -f "$f" ] || continue
    base=$(basename "$f")
    [ -e "$d"lib/"$base" ] || cp "$f" "$d"lib/"$base" 2>/dev/null || true
    [ -e "$d"lib64/"$base" ] || cp "$f" "$d"lib64/"$base" 2>/dev/null || true
  done
done
echo "rootfs ld-linux:" | tee -a "$OUT_DIR/probe.log"
find "$ROOTFS" -name 'ld-linux*' -ls | tee -a "$OUT_DIR/probe.log"
EOF
awk 'FNR == NR { block = block $0 "\n"; next } /echo "rootfs files:"/ && !done { printf "%s", block; done = 1 } { print }' /tmp/kata-ptinterp-block /etc/kata-debug/guest/02-run-rootfs-probe.sh > /tmp/02-run-rootfs-probe.sh.new && mv /tmp/02-run-rootfs-probe.sh.new /etc/kata-debug/guest/02-run-rootfs-probe.sh; fi"##,
    )?;
    Ok(())
}

fn run_kata_debug(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(
        "timeout 180s kata-debug-run-all > /tmp/kata-debug-run-all.stdout 2>&1; \
         echo run-all-exit:$? >> /tmp/kata-debug-run-all.stdout",
    )
}

/// Smoke test: the container runs `/bin/sh -c '...; exit 0'`, writes a file
/// inside its rootfs, and exits 0. Validates the full
/// containerd → shim-kata-v2 → runtime-rs → nested QEMU → kata-agent path
/// plus host vhost-vsock control + data plane.
#[nixos_test]
fn kata_rootfs_run_probe(nixos_shell: &mut Session) -> Result<(), Error> {
    prepare_rootfs_probe(nixos_shell, "echo SHRAN > /testout.txt; exit 0")?;
    run_kata_debug(nixos_shell)?;
    nixos_shell.run_cmd_and_expect("cat /tmp/kata-run.txt", "exit:0")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/kata-rootfs/testout.txt", "SHRAN")?;
    Ok(())
}

/// Validates that a container exit code other than zero propagates back
/// through ttrpc → kata-agent → runtime-rs → shim → ctr, and that the
/// container's stdout is captured into ctr's output stream. Exercises the
/// vhost-vsock data plane in both directions with a non-trivial payload.
#[nixos_test]
fn kata_rootfs_exit_code_propagation(nixos_shell: &mut Session) -> Result<(), Error> {
    // Multi-step shell using only bash builtins (the rootfs has only bash;
    // `cat` etc. aren't ldd-copied). Writes a file, prints to stdout, exits non-zero.
    prepare_rootfs_probe(
        nixos_shell,
        "echo HI-FROM-CTR > /testout.txt; echo HI-FROM-CTR; exit 7",
    )?;
    run_kata_debug(nixos_shell)?;
    // ctr propagates the container's exit code as its own; the script
    // appends `exit:$STATUS` to /tmp/kata-run.txt.
    nixos_shell.run_cmd_and_expect("cat /tmp/kata-run.txt", "exit:7")?;
    // ctr captures container stdout into the same file.
    nixos_shell.run_cmd_and_expect("cat /tmp/kata-run.txt", "HI-FROM-CTR")?;
    // The file write inside the container's rootfs is also visible on the host.
    nixos_shell.run_cmd_and_expect("cat /tmp/kata-rootfs/testout.txt", "HI-FROM-CTR")?;
    Ok(())
}
