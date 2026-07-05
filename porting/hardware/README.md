# Hardware-Specific Boot Artifacts

This directory contains boot assembly, linker scripts, and patches used while
bringing up RISC-V code on the **Milk-V Megrez** (ESWIN EIC7700) board.

These files are **not** part of the main Asterinas build system. They are
experiments, fallback versions, and reference snippets accumulated during the
port. The canonical boot code for this repo is now
`ostd/src/arch/riscv/boot/boot.S`.

## Files

| File | Purpose |
|------|---------|
| [`boot_megrez.S`](boot_megrez.S) | Early Milk-V Megrez BSP entrypoint. Similar in spirit to the current `ostd/src/arch/riscv/boot/boot.S` but with different label/section names. Kept for reference. |
| [`boot_test.S`](boot_test.S) | Minimal self-contained Sv39 boot test for QEMU `virt`; useful for validating page-table setup without the full kernel. |
| [`bsp_boot_orig.S`](bsp_boot_orig.S) | Original BSP boot assembly before the `PTE_A`/`PTE_D` debug changes were added. |
| [`bsp_boot_debug.S`](bsp_boot_debug.S) | Standalone debug version of the BSP boot code with MMIO UART markers and a tiny trap handler. |
| [`bsp_boot_debug.patch`](bsp_boot_debug.patch) | Diff adding `PTE_A`/`PTE_D` definitions to the original boot code. |
| [`linker.ld`](linker.ld) | Reference linker script with `KERNEL_LMA = 0x80200000` and high-half VMA mapping. |
| [`linker_m.ld`](linker_m.ld) | Minimal linker script that loads at `0x80000000`. |
| [`linker_simple.ld`](linker_simple.ld) | Minimal linker script that loads at `0x80200000`. |
| [`uart_test.S`](uart_test.S) | Tiny NS16550-style UART transmit test (QEMU `virt` base `0x10000000`). |

## Using these files

Most of these are one-off experiments. If you want to rebuild a standalone boot
test, for example:

```bash
riscv64-linux-gnu-gcc -T porting/hardware/linker_simple.ld \
    -nostdlib -o /tmp/boot_test.elf porting/hardware/boot_test.S
```

For the current Asterinas build path, see [`porting/boot.md`](../boot.md).
