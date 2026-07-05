# Local Testing Before Board Bring-Up

This document describes how to test as much of the Milk-V Megrez / Asterinas
port as possible **without touching the real board**. Local testing cannot
reproduce EIC7700-specific details, but it catches build errors, generic RISC-V
boot mistakes, and `booti` image format problems early.

## What can and cannot be simulated locally

| Aspect | Local (QEMU) | Real Milk-V Megrez |
|--------|--------------|--------------------|
| Build / dependency errors | ✅ `cargo osdk build` | same |
| Generic RISC-V MMU / trap / boot flow | ✅ `cargo osdk run` | same |
| Early boot markers on serial | ✅ after UART remap | same, but at `0x50900000` |
| `booti` image format / U-Boot load | ⚠️ needs self-built U-Boot | ✅ native |
| Megrez DTB / memory layout | ❌ QEMU `virt` DTB | ✅ real DTB |
| Megrez UART (`0x50900000`) | ❌ no | ✅ yes |
| Megrez PLIC / timer / cache quirks | ❌ no | ✅ yes |

## Prerequisites

```bash
# 1. Rust toolchain (managed by rust-toolchain.toml)
# 2. cargo-osdk installed
# 3. linux_vdso cloned
export OSDK_LOCAL_DEV=1
export VDSO_LIBRARY_DIR=$HOME/linux_vdso

# 4. A placeholder initramfs (already present in this tree)
ls test/build/initramfs.cpio.gz
```

## 1. Build for QEMU (`riscv` scheme)

```bash
cargo osdk build --scheme riscv --target-arch riscv64
```

Expected result: the ELF is produced at
`target/riscv64gc-unknown-none-elf/debug/aster-nix-osdk-bin`.

## 2. Build for the real board (`milkv-megrez` scheme)

```bash
cargo osdk build --scheme milkv-megrez --target-arch riscv64
```

This uses the `[scheme."milkv-megrez"]` entry in `OSDK.toml`. It produces the
same ELF but records that the image is intended for U-Boot on the board.

## 3. Run in QEMU with visible early markers

The real Megrez UART is at `0x50900000`; QEMU `virt` uses NS16550 at
`0x10000000`. Use the helper that temporarily remaps the UART base:

```bash
./porting/scripts/qemu_run_megrez.sh 45
```

The script:

1. Backs up `ostd/src/arch/riscv/boot/boot.S`.
2. Replaces `li t3, 0x50900000` with `li t3, 0x10000000`.
3. Builds with `cargo osdk build --scheme riscv --target-arch riscv64`.
4. Runs `qemu-system-riscv64 -machine virt ... -kernel <elf>` directly.
5. Restores the original `boot.S` on exit.

Output is saved to `/tmp/aster-qemu-run.log`. In the log you should see the
marker sequence from `boot.S`, for example:

```text
AEFGDHB C...
```

> If the script exits with code 124, that is the expected `timeout` result.

## 4. Generate and inspect the U-Boot `booti` image

```bash
python3 porting/scripts/mkimage.py \
    target/riscv64gc-unknown-none-elf/debug/aster-nix-osdk-bin \
    aster-nix.booti

ls -lh aster-nix.booti
```

The helper `mkimage.py` prepends the 64-byte RISC-V Linux Image header that
U-Boot's `booti` command expects.

## 5. (Optional) Test the `booti` image with U-Boot in QEMU

This validates the exact load-and-boot path used on the real board.

### 5.1 Build U-Boot for QEMU

```bash
git clone https://github.com/u-boot/u-boot
cd u-boot
make qemu-riscv64_smode_defconfig
make -j$(nproc)
```

> Requires `bison`, `flex`, and `bc`. If these are missing the build will fail
> at the kconfig step.

### 5.2 Create a boot image

```bash
mkdir -p /tmp/megrez-boot
cp aster-nix.booti /tmp/megrez-boot/
qemu-system-riscv64 -machine virt -m 128M -machine dumpdtb=/tmp/megrez-boot/qemu-virt.dtb
mkfs.ext4 -d /tmp/megrez-boot -L boot /tmp/megrez-boot.img 64M
```

### 5.3 Run U-Boot in QEMU

```bash
qemu-system-riscv64 -machine virt -m 2G -nographic \
    -bios /tmp/u-boot/u-boot.bin \
    -drive file=/tmp/megrez-boot.img,format=raw,id=sd0 \
    -device virtio-blk-device,drive=sd0
```

At the U-Boot prompt:

```bash
ext4load virtio 0:1 0x80200000 /aster-nix.booti
ext4load virtio 0:1 0xf0000000 /qemu-virt.dtb
booti 0x80200000 - 0xf0000000
```

Because QEMU `virt` peripherals differ from Megrez, the kernel is likely to
hang once it tries to use Megrez-specific devices. The value here is verifying
that `booti` accepts the image and that `_start` runs.

## 6. Actual local test results

Using the current tree:

- `cargo osdk build --scheme riscv --target-arch riscv64` ✅ succeeds
- `cargo osdk build --scheme milkv-megrez --target-arch riscv64` ✅ succeeds
- `./porting/scripts/qemu_run_megrez.sh 45` ✅ reaches the MMU-enabled boot
  markers in QEMU:

  ```text
  AEFGDHB
  ```

  This confirms the page-table setup, SATP write/read, Sv48 enable, and high-half
  jump work on a generic RISC-V machine.

- `python3 porting/scripts/mkimage.py ... /tmp/aster-nix.booti` ✅ produces a
  6.9 MiB image with the correct RISC-V Linux Image header:

  | Offset | Value | Meaning |
  |--------|-------|---------|
  | 0x00   | `0x0000006f` | `j 0x40` |
  | 0x08   | `0x00200000` | `text_offset` |
  | 0x38   | `0x05435352` | RISC-V Image magic |

- U-Boot-in-QEMU test ⚠️ blocked in this environment because the build
  dependencies `bison`, `flex`, and `bc` are not installed and `sudo` is not
  passwordless. The commands above are ready to run once those tools are
  available.

## 6. Known local-only differences to keep in mind

- **UART base:** `0x10000000` in QEMU, `0x50900000` on Megrez.
- **DTB source:** QEMU generates its own; Megrez uses
  `/dtbs/linux-image-6.6.87-win2030/eswin/eic7700-milkv-megrez.dtb`.
- **I/O devices:** QEMU exposes virtio; Megrez has on-chip UART, PLIC, ethernet,
  SD/eMMC, etc.
- **SBI vs U-Boot:** QEMU uses OpenSBI; Megrez enters S-mode from U-Boot.
  Early `mhartid` reads trap on Megrez but are safe under OpenSBI.

## Recommended workflow

1. Edit code.
2. `cargo osdk build --scheme riscv --target-arch riscv64` (fast local build check).
3. `./porting/scripts/qemu_run_megrez.sh 45` (check early boot markers).
4. `cargo osdk build --scheme milkv-megrez --target-arch riscv64` (board build).
5. `python3 porting/scripts/mkimage.py ... aster-nix.booti`.
6. Copy `aster-nix.booti` to the board and test on real hardware.

See also:

- [`boot.md`](boot.md) — board-side U-Boot commands.
- [`issues/`](issues/) — issue records for problems found in local and board
  testing.
