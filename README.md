# Asterinas RISC-V Port for Milk-V Megrez

This repository is a working tree for porting [Asterinas](https://github.com/asterinas/asterinas) to the **Milk-V Megrez** development board, which is based on the **ESWIN EIC7700** RISC-V SoC.

## Repository Layout

This repo has two main areas:

1. **Asterinas source code** — located at the repository root (`kernel/`, `ostd/`, `osdk/`, `test/`, `tools/`, etc.). This is the upstream-derived codebase being ported.
2. **Porting notes and tooling** — located under [`porting/`](porting/). This contains setup guides, boot recipes, issue records, helper scripts, and hardware-specific artifacts accumulated during the port.

## Quick Start

### Build the RISC-V Kernel

```bash
export OSDK_LOCAL_DEV=1
cargo osdk build --scheme riscv --target-arch riscv64
```

### Create a U-Boot Bootable Image

```bash
python3 porting/scripts/mkimage.py \
    target/riscv64gc-unknown-none-elf/debug/aster-nix-osdk-bin \
    aster-nix.booti
```

### Boot on the Board

See [`porting/boot.md`](porting/boot.md) for the full U-Boot command sequence.

## Documentation

| Document | Purpose |
|----------|---------|
| [`porting/README.md`](porting/README.md) | Porting project overview and status |
| [`porting/setup.md`](porting/setup.md) | Serial, network, SSH, and Linux boot setup |
| [`porting/boot.md`](porting/boot.md) | Building and booting Asterinas on Megrez |
| [`porting/testing.md`](porting/testing.md) | Local testing before touching the real board |
| [`porting/issues/`](porting/issues/) | Issue-by-issue debugging records |
| [`porting/scripts/`](porting/scripts/) | Build and automation helpers |
| [`porting/hardware/`](porting/hardware/) | Boot/linker source files for the board |

## Current Status

- ✅ Serial console access via FTDI USB-UART
- ✅ Debian Linux boots from SD card
- ✅ Ethernet and SSH key login work through Windows ICS
- ✅ Asterinas builds for RISC-V
- ✅ Dedicated `[scheme."milkv-megrez"]` for board builds
- ✅ Local QEMU run reaches the MMU-enabled boot markers (`AEFGDHB`)
- ✅ `booti` image generation produces a valid RISC-V Linux Image header
- ✅ Asterinas `booti` image loads in U-Boot
- 🔄 **In progress:** getting Asterinas to run past early MMU setup on real hardware

See [`porting/issues/03-asterinas-mmu-hang-at-AEFGD.md`](porting/issues/03-asterinas-mmu-hang-at-AEFGD.md) for the latest blocker.

## License

Asterinas's source code and documentation primarily use the
[Mozilla Public License (MPL), Version 2.0](LICENSE-MPL).
Porting notes and scripts in `porting/` follow the same license unless otherwise
noted.
