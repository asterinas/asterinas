#!/bin/bash
# Run Asterinas in QEMU's RISC-V virt machine with UART remapped so that the
# Milk-V Megrez early boot markers are visible on the emulated serial console.
#
# The real Megrez UART is at 0x50900000; QEMU virt uses NS16550 at 0x10000000.
# This script temporarily patches ostd/src/arch/riscv/boot/boot.S, runs the
# kernel, and restores the original file on exit.
#
# Usage:
#   ./porting/scripts/qemu_run_megrez.sh [timeout_seconds]
#
# Example:
#   ./porting/scripts/qemu_run_megrez.sh 30

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BOOT_S="$REPO_ROOT/ostd/src/arch/riscv/boot/boot.S"
BOOT_S_ORIG="$BOOT_S.qemu-orig"

REAL_UART="0x50900000"
QEMU_UART="0x10000000"

TIMEOUT="${1:-30}"

if [ -z "${VDSO_LIBRARY_DIR:-}" ]; then
    echo "[!] VDSO_LIBRARY_DIR is not set." >&2
    echo "    Clone https://github.com/asterinas/linux_vdso and export VDSO_LIBRARY_DIR." >&2
    exit 1
fi

cleanup() {
    if [ -f "$BOOT_S_ORIG" ]; then
        mv "$BOOT_S_ORIG" "$BOOT_S"
        echo "[+] Restored $BOOT_S"
    fi
}
trap cleanup EXIT

# Backup original boot.S
if [ -f "$BOOT_S_ORIG" ]; then
    echo "[!] Backup already exists ($BOOT_S_ORIG); another instance may be running." >&2
    exit 1
fi
cp "$BOOT_S" "$BOOT_S_ORIG"

# Replace the Milk-V UART base with the QEMU virt UART base.
sed -i "s/li t3, $REAL_UART/li t3, $QEMU_UART/g" "$BOOT_S"
if grep -q "li t3, $REAL_UART" "$BOOT_S"; then
    echo "[!] Failed to replace all UART load-immediate instructions in $BOOT_S" >&2
    exit 1
fi
echo "[+] Remapped UART $REAL_UART -> $QEMU_UART in $BOOT_S"

cd "$REPO_ROOT"
export OSDK_LOCAL_DEV=1

echo "[+] Building for QEMU ..."
cargo osdk build --scheme riscv --target-arch riscv64 2>&1 | tee /tmp/aster-qemu-run.log

ELF="$REPO_ROOT/target/riscv64gc-unknown-none-elf/debug/aster-nix-osdk-bin"
if [ ! -f "$ELF" ]; then
    echo "[!] Kernel ELF not found: $ELF" >&2
    exit 1
fi

echo "[+] Running QEMU (timeout ${TIMEOUT}s) ..."
set +e
timeout --foreground "$TIMEOUT" \
    qemu-system-riscv64 \
        -machine virt \
        -m 8G \
        -nographic \
        -bios default \
        -kernel "$ELF" \
    2>&1 | tee -a /tmp/aster-qemu-run.log
EXIT_CODE=$?
set -e

if [ "$EXIT_CODE" -eq 124 ]; then
    echo "[+] QEMU run timed out after ${TIMEOUT}s (expected). See /tmp/aster-qemu-run.log"
elif [ "$EXIT_CODE" -ne 0 ]; then
    echo "[!] QEMU run failed with exit code $EXIT_CODE. See /tmp/aster-qemu-run.log" >&2
fi

exit "$EXIT_CODE"
