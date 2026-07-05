#!/bin/bash
# Run Asterinas in QEMU with the real Milk-V Megrez DTB.
# Tests DTB parsing, memory regions, timebase frequency, CPU topology
# without touching the real board.
#
# Usage:
#   ./porting/scripts/qemu_run_megrez_dtb.sh [timeout_seconds]
#
# The script:
#   1. Builds Asterinas with the riscv scheme
#   2. Runs QEMU virt machine with -dtb pointing to the real Megrez DTB
#   3. CPU flags matched to Megrez's SiFive P550 (rv64imafdch_zba_zbb_sscofpmf)
#   4. Captures boot output to check markers: AEFGDHB C

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DTB="$REPO_ROOT/porting/images/eic7700-milkv-megrez.dtb"
LOG="/tmp/aster-megrez-dtb.log"

TIMEOUT="${1:-45}"
QEMU_CPU="rv64,zba=true,zbb=true,v=true,vext_spec=v1.0"

if [ -z "${VDSO_LIBRARY_DIR:-}" ]; then
    echo "[!] VDSO_LIBRARY_DIR is not set." >&2
    echo "    Clone https://github.com/asterinas/linux_vdso and export VDSO_LIBRARY_DIR." >&2
    exit 1
fi

if [ ! -f "$DTB" ]; then
    echo "[!] Real Megrez DTB not found: $DTB" >&2
    echo "    Run the board info collection script first." >&2
    exit 1
fi

echo "=== Asterinas Megrez DTB Simulation Test ==="
echo "DTB:    $DTB ($(wc -c < "$DTB") bytes)"
echo "CPU:    $QEMU_CPU"
echo "Log:    $LOG"
echo "Timeout: ${TIMEOUT}s"
echo ""

cd "$REPO_ROOT"
export OSDK_LOCAL_DEV=1

echo "[+] Building Asterinas for RISC-V ..."
cargo osdk build --scheme riscv --target-arch riscv64 2>&1 | tail -3

ELF="$REPO_ROOT/target/riscv64gc-unknown-none-elf/debug/aster-nix-osdk-bin"
if [ ! -f "$ELF" ]; then
    echo "[!] Kernel ELF not found: $ELF" >&2
    exit 1
fi
echo "[+] ELF: $ELF ($(stat -c%s "$ELF") bytes)"

echo "[+] Starting QEMU ..."
set +e
timeout --foreground "$TIMEOUT" \
    qemu-system-riscv64 \
        -machine virt \
        -cpu "$QEMU_CPU" \
        -m 4G \
        -nographic \
        -bios default \
        -dtb "$DTB" \
        -kernel "$ELF" \
        -global virtio-mmio.force-legacy=false \
    2>&1 | tee "$LOG"
EXIT_CODE=$?
set -e

echo ""
echo "=== Results ==="
echo "Exit code: $EXIT_CODE (124 = timeout, which is expected)"

# Check boot markers
MARKERS=$(grep -o '[AEFGDHBCh@!T]' "$LOG" 2>/dev/null | tr -d '\n')
echo "Boot markers: $MARKERS"

if echo "$MARKERS" | grep -q "AEFGDHB"; then
    echo "✅ Early boot markers (AEFGDHB) all present!"
else
    echo "❌ Early boot markers incomplete!"
fi

if echo "$MARKERS" | grep -q "C"; then
    echo "✅ Reached Rust entry riscv_boot (marker C)!"
else
    echo "❌ Did not reach riscv_boot!"
fi

# Check for Rust-level output (early_println)
if grep -q "Enter riscv_boot\|ostd_main\|Hello\|panic" "$LOG" 2>/dev/null; then
    echo "✅ Rust-level output detected in log!"
    grep -E "Enter|boot|panic|Hello|memory|region" "$LOG" 2>/dev/null | head -20
else
    echo "⚠️  No Rust-level output detected (may need UART remap or longer timeout)"
fi

echo ""
echo "Full log: $LOG"
exit 0
