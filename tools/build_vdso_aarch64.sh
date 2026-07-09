#!/bin/bash
# SPDX-License-Identifier: MPL-2.0
#
# Rebuilds `kernel/src/vdso_aarch64.so`, the AArch64 vDSO.
#
# The vDSO is a tiny hand-written assembly shared object provided by the kernel
# to userspace. It contains no C and links no libc — it is assembled from a
# `#![no_std]` Rust source (`global_asm!`) and linked with `rust-lld`. It is
# built once and checked in because it is a stable, architecture-fixed artifact.
#
# After rebuilding, update `__VDSO_RT_SIGRETURN_OFFSET` in `kernel/src/vdso.rs`
# with the value printed at the end.

set -e

ASTER_DIR="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ASTER_DIR/kernel/src/vdso_aarch64.so"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"

cat > vdso.rs <<'EOF'
#![no_std]
#![no_main]
core::arch::global_asm!(
    ".section .text",
    ".balign 16",
    ".global __vdso_rt_sigreturn",
    ".global __kernel_rt_sigreturn",
    ".type __vdso_rt_sigreturn, %function",
    "__vdso_rt_sigreturn:",
    "__kernel_rt_sigreturn:",
    "    mov x8, #139", // __NR_rt_sigreturn
    "    svc #0",
);
#[panic_handler]
fn ph(_: &core::panic::PanicInfo) -> ! { loop {} }
EOF

cat > vdso.ver <<'EOF'
LINUX_2.6.39 {
global:
  __vdso_rt_sigreturn;
  __kernel_rt_sigreturn;
local: *;
};
EOF

rustc --target aarch64-unknown-none-softfloat --edition 2021 --emit obj -O \
    -C panic=abort -o vdso.o vdso.rs

LLD="$(find "${RUSTUP_HOME:-$HOME/.rustup}" -name rust-lld | head -1)"
"$LLD" -flavor gnu -shared -Bsymbolic --build-id=none --hash-style=sysv \
    --version-script=vdso.ver -soname=linux-vdso.so.1 \
    -z max-page-size=4096 -z noseparate-code --no-rosegment \
    -o "$OUT" vdso.o

# The kernel copies a full page of the vDSO as its text segment, so pad the
# (sub-page) shared object up to a page boundary.
echo -n "__VDSO_RT_SIGRETURN_OFFSET = "
readelf -sW "$OUT" | awk '/__vdso_rt_sigreturn$/ {print "0x"$2; exit}'
truncate -s 4096 "$OUT"
echo "Wrote $OUT (padded to 4096 bytes)"
