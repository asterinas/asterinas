# Asterinas on Milk-V Megrez (ESWIN EIC7700)

This directory contains everything related to porting and running Asterinas on the
[Milk-V Megrez](https://milkv.io/megrez) development board (ESWIN EIC7700, RISC-V 64).

The Asterinas source code itself lives in the repository root (`kernel/`, `ostd/`,
`osdk/`, etc.). This `porting/` folder is the second top-level area: it holds
setup guides, boot recipes, issue records, helper scripts, and hardware-specific
artifacts accumulated during the port.

## Quick Links

| Topic | File |
|-------|------|
| Hardware setup, serial, network, SSH | [`setup.md`](setup.md) |
| Booting Linux and Asterinas from U-Boot | [`boot.md`](boot.md) |
| Local testing before board bring-up | [`testing.md`](testing.md) |
| Issue-by-issue debugging notes | [`issues/`](issues/) |
| Helper scripts (PowerShell / Python / Bash) | [`scripts/`](scripts/) |
| Boot/linker hardware artifacts | [`hardware/`](hardware/) |
| Sample boot logs | [`logs/`](logs/) |
| External references and datasheets | [`references/`](references/) |

## Current Status

- ✅ U-Boot serial console accessible via FTDI USB-UART at 115200 baud
- ✅ Debian Linux boots from SD card (`mmc 1`)
- ✅ Ethernet `end1` works with Windows Internet Connection Sharing
- ✅ SSH key login from Windows to the board works
- ✅ Asterinas builds for RISC-V (`cargo osdk build --scheme riscv --target-arch riscv64`)
- ✅ Dedicated `[scheme."milkv-megrez"]` in `OSDK.toml` for board builds
- ✅ Local QEMU run reaches the MMU-enabled boot markers (`AEFGDHB`)
- ✅ `booti` image generation produces a valid RISC-V Linux Image header
- ✅ Asterinas `booti` image can be loaded by U-Boot
- 🔄 **In progress:** booting Asterinas on real hardware. Latest attempt hangs
  after printing `AEFGD` (see [`issues/03-asterinas-mmu-hang-at-AEFGD.md`](issues/03-asterinas-mmu-hang-at-AEFGD.md)).

## Hardware at a Glance

| Component | Detail |
|-----------|--------|
| Board | Milk-V Megrez |
| SoC | ESWIN EIC7700X (SiFive P550 cores) |
| CPU | RISC-V 64, `rv64imafdch_zicsr_zifencei_zba_zbb_sscofpmf` |
| RAM | 16 GB LPDDR5 |
| Storage | SanDisk 128 GB microSD (Debian) |
| Serial | FTDI USB-UART, 115200 8N1, on `/dev/ttyUSB0` in WSL |
| Ethernet | `end1` via Windows ICS, board IP `192.168.100.2/24` |

## Building the RISC-V Kernel

From the repository root:

```bash
export OSDK_LOCAL_DEV=1
cargo osdk build --scheme riscv --target-arch riscv64
```

The resulting ELF is at:

```text
target/riscv64gc-unknown-none-elf/debug/aster-nix-osdk-bin
```

To make a U-Boot `booti` image:

```bash
python3 porting/scripts/mkimage.py \
    target/riscv64gc-unknown-none-elf/debug/aster-nix-osdk-bin \
    aster-nix.booti
```

Then copy `aster-nix.booti` to the board's `/boot` partition and boot it from
U-Boot (details in [`boot.md`](boot.md)).

## Layout Convention

- One issue per file under `issues/`, named `NN-short-description.md`.
- Each issue file follows the template: **Symptom → Cause → Fix → Verification**.
- `scripts/` contains cross-platform helpers; PowerShell scripts are prefixed
  with `ps-` to make the runtime obvious.
- `hardware/` contains boot/linker source files that are specific to this board.
- `logs/` contains trimmed, representative boot logs (large or binary artifacts
  are gitignored).
