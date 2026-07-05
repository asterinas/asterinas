# Porting Scripts

This directory contains helper scripts used during the Milk-V Megrez port.

## Core Build Helpers

| Script | Purpose |
|--------|---------|
| [`mkimage.py`](mkimage.py) | Convert the Asterinas RISC-V ELF into a U-Boot `booti`-compatible image. |
| [`deploy_debug.sh`](deploy_debug.sh) | Build Asterinas and regenerate the booti image. Run as `deploy_debug.sh patch` to copy the debug boot payload from `porting/hardware/bsp_boot_debug.S` before building. |
| [`qemu_run_megrez.sh`](qemu_run_megrez.sh) | Temporarily remap the early UART to QEMU virt's NS16550 base and run `cargo osdk run` locally. |
| [`setup-rust.sh`](setup-rust.sh) | Install the Rust toolchain for this port. |

## Board-Specific Helpers

| Script | Purpose |
|--------|---------|
| [`patch_bsp.py`](patch_bsp.py) | Patch the BSP boot assembly in `ostd/src/arch/riscv/boot/`. |
| [`patch_rootfs.py`](patch_rootfs.py) | Patch the Debian root filesystem image. |
| [`fix_atomic*.py`](fix_atomic.py) | Workarounds for atomic-related issues encountered during bring-up. |
| [`mktest_libflate.py`](mktest_libflate.py) | Helper for initramfs/libflate testing. |

## Windows PowerShell Serial Automation

Scripts prefixed with `ps-` are PowerShell helpers that run on Windows and talk
to the FTDI serial port. They were used for automated reboot, autoboot
interruption, command sending, and log capture before the WSL/Python workflow
was established.

| Script | Purpose |
|--------|---------|
| `ps-serial_stop_autoboot.ps1` | Send a key during U-Boot countdown. |
| `ps-serial_boot.ps1` / `ps-serial_boot2.ps1` | Send Linux/Asterinas boot commands. |
| `ps-serial_login_reboot.ps1` | Log into Debian and reboot. |
| `ps-xmodem_send.ps1` | Send files over serial (slow fallback). |
| `ps-serial_capture.ps1` | Capture serial output to a file. |

> The PowerShell scripts contain hardcoded COM ports and paths from the original
> Windows environment. Review and adjust before reuse.
