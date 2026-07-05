#!/bin/bash
# Build and optionally patch Asterinas for Milk-V Megrez debug bring-up.
#
# Default mode is "build" (no source patching). Run with "patch" to replace the
# BSP boot assembly with the standalone debug payload in
# porting/hardware/bsp_boot_debug.S before building.
#
# All paths can be overridden with environment variables:
#   ASTERINAS_DIR      - repo root (default: parent of this script's repo)
#   BSP_BOOT           - boot assembly file to patch (default: .../bsp_boot.S)
#   OUT_BIN            - kernel ELF used by mkimage.py
#   OUT_BOOTI          - generated U-Boot booti image
#   MKIMAGE            - path to mkimage.py
#   DEBUG_BOOT_SRC     - debug payload to copy in patch mode
#   VDSO_LIBRARY_DIR   - required, see porting/issues/04-cargo-osdk-missing-vdso.md

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

ASTERINAS_DIR="${ASTERINAS_DIR:-$REPO_ROOT}"
BSP_BOOT="${BSP_BOOT:-$ASTERINAS_DIR/ostd/src/arch/riscv/boot/bsp_boot.S}"
OUT_BIN="${OUT_BIN:-$REPO_ROOT/target/riscv64gc-unknown-none-elf/debug/aster-nix-osdk-bin}"
OUT_BOOTI="${OUT_BOOTI:-$REPO_ROOT/aster-nix.booti}"
MKIMAGE="${MKIMAGE:-$REPO_ROOT/porting/scripts/mkimage.py}"
DEBUG_BOOT_SRC="${DEBUG_BOOT_SRC:-$REPO_ROOT/porting/hardware/bsp_boot_debug.S}"

MODE="${1:-build}"

usage() {
    echo "Usage: $0 [build|patch]" >&2
    echo "  build  - build the kernel and regenerate the booti image (default)" >&2
    echo "  patch  - copy the debug boot payload, then build" >&2
}

case "$MODE" in
    build|patch) ;;
    *) usage; exit 1 ;;
esac

if [ -z "${VDSO_LIBRARY_DIR:-}" ]; then
    echo "[!] VDSO_LIBRARY_DIR is not set." >&2
    echo "    Clone https://github.com/asterinas/linux_vdso and export VDSO_LIBRARY_DIR." >&2
    echo "    See porting/issues/04-cargo-osdk-missing-vdso.md" >&2
    exit 1
fi

if [ "$MODE" = "patch" ]; then
    if [ ! -f "$DEBUG_BOOT_SRC" ]; then
        echo "[!] Debug boot payload not found: $DEBUG_BOOT_SRC" >&2
        exit 1
    fi

    echo "[+] Target: $BSP_BOOT"

    # Backup original (only once)
    if [ ! -f "$BSP_BOOT.orig" ]; then
        cp "$BSP_BOOT" "$BSP_BOOT.orig"
        echo "[+] Backed up original to $BSP_BOOT.orig"
    fi

    cp "$DEBUG_BOOT_SRC" "$BSP_BOOT"
    echo "[+] Patched $BSP_BOOT with $DEBUG_BOOT_SRC"
fi

echo "[+] Building in $ASTERINAS_DIR"
cd "$ASTERINAS_DIR"
export VDSO_LIBRARY_DIR
export OSDK_LOCAL_DEV=1
cargo osdk build --scheme riscv --target-arch riscv64

echo "[+] Build complete"

# Repackage.
python3 "$MKIMAGE" "$OUT_BIN" "$OUT_BOOTI"
echo "[+] Generated $OUT_BOOTI"

echo ""
echo "Now on the U-Boot prompt run:"
echo "    booti 0x80200000 - 0xf0000000"
echo ""
echo "Expected markers after fix: AEFGDHB C..."
echo "If it stops after AEF, satp is inaccessible (TVM/U-mode)."
echo "If it prints AEFGDH then T<c/d/f/2>, a translation fault still occurs."
