#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

# Regression test for the loop-partition readiness race (issue #3700):
# repeat the aster-nixos-install template's provisioning path
# (losetup -> parted -> mkfs) on a fresh loop disk. The unlocked installer
# failed within 24-240 iterations, so a single install cannot catch it.
# A PATH shim replaces mount, so every run aborts right after
# "mkfs finished" and one iteration costs well under a second instead of
# a full nixos-install.

set -Eeuo pipefail

# Run 500 iterations (losetup -> parted -> mkfs) on a fresh loop disk
STRESS_RUNS=500
MOUNT_SHIM_EXIT=42

trap 'echo "::error::Stress harness failed at line $LINENO" >&2; exit 2' ERR

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ASTERINAS_DIR=$(realpath "${SCRIPT_DIR}/../..")
CONFIG_PATH="${ASTERINAS_DIR}/distro/etc_nixos/configuration.nix"
# Nix substitutions are only used after the mount shim aborts the installer.
INSTALLER="${ASTERINAS_DIR}/distro/aster_nixos_installer/templates/aster-nixos-install"

stress_tmp=$(mktemp -d /tmp/installer-provisioning-stress.XXXXXX)
stress_disk=""
stress_image=""

release_iteration() {
    local absence_streak=0
    local disk=$stress_disk
    local _attempt

    if [ -n "$disk" ]; then
        losetup -d "$disk" || return 1
        stress_disk=""
        # Detach is asynchronous too: wait for stable node absence so the
        # next iteration cannot reuse a loop device that is still dying.
        for ((_attempt = 0; _attempt < 500; _attempt++)); do
            if [ ! -e "${disk}p1" ] && [ ! -e "${disk}p2" ]; then
                absence_streak=$((absence_streak + 1))
                if [ "$absence_streak" -ge 5 ]; then
                    break
                fi
            else
                absence_streak=0
            fi
            sleep 0.01
        done
        if [ "$absence_streak" -lt 5 ]; then
            return 1
        fi
    fi
    rm -f "$stress_image"
}

cleanup_all() {
    local status=$?
    trap - EXIT
    if ! release_iteration; then
        echo "::error::Cannot release loop device $stress_disk" >&2
        if [ "$status" -eq 0 ]; then status=2; fi
    fi
    if ! rm -rf "$stress_tmp"; then
        if [ "$status" -eq 0 ]; then status=2; fi
    fi
    exit "$status"
}
trap cleanup_all EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Abort at mount to avoid running a full installation
mkdir -p "$stress_tmp/shim"
cat > "$stress_tmp/shim/mount" <<EOF
#!/bin/sh
exit $MOUNT_SHIM_EXIT
EOF
# Redirect temporary directories into $stress_tmp for cleanup, since the
# installer exits before registering its own cleanup path
chmod +x "$stress_tmp/shim/mount"
cat > "$stress_tmp/shim/mktemp" <<EOF
#!/usr/bin/env bash
exec $(printf '%q -d %q' "$(command -v mktemp)" "$stress_tmp/build.XXXXXX")
EOF
chmod +x "$stress_tmp/shim/mktemp"

echo "runs=$STRESS_RUNS installer=$(readlink -f "$INSTALLER")"

for ((iteration = 1; iteration <= STRESS_RUNS; iteration++)); do
    stress_image="$stress_tmp/disk-$iteration.img"
    # Sparse image: the hard-coded partition layout needs >= 1GB of
    # address space but only a few MB are ever written.
    if ! truncate -s 1024M "$stress_image"; then
        echo "iteration=$iteration operation=truncate failure=harness"
        exit 2
    fi
    if ! stress_disk=$(losetup -fP --show "$stress_image"); then
        echo "iteration=$iteration operation=losetup failure=harness"
        exit 2
    fi

    status=0
    output=$(PATH="$stress_tmp/shim:$PATH" \
        bash "$INSTALLER" --config "$CONFIG_PATH" --disk "$stress_disk" 2>&1) || status=$?

    if [ "$status" -ne "$MOUNT_SHIM_EXIT" ] ||
        [[ "$output" != *"mkfs finished"* ]]; then
        echo "iteration=$iteration disk=$stress_disk status=$status failure=provisioning"
        printf '%s\n' "$output"
        exit 1
    fi

    if ! release_iteration; then
        echo "iteration=$iteration operation=loop-cleanup failure=harness"
        exit 2
    fi
    if [ $((iteration % 50)) -eq 0 ]; then
        echo "progress completed=$iteration"
    fi
done

echo "Installer provisioning survived $STRESS_RUNS runs"
