# Issue 12: Expected blockers for real-hardware bring-up

This issue collects hardware-specific problems that have not happened yet but
are expected while moving from QEMU to the Milk-V Megrez board.

## Expected problems

1. **Entry point / load address mismatch**

   QEMU's `sifive_u` scheme loads the kernel at `0x80200000`. U-Boot on Megrez
   may load the image differently depending on whether `booti`, `bootm`, `go`, or
   `bootelf` is used. The link address in the OSDK linker script may need to be
   adjusted.

2. **UART base address difference**

   QEMU `virt` uses `0x10000000`; Megrez uses `snps,dw-apb-uart` at
   `0x50900000`. Early serial output may be silent until the Megrez UART driver
   is initialized.

3. **PLIC vs AIA interrupt controller**

   Megrez's DTB reports a `sifive,plic-1.0.0` compatible PLIC, which Asterinas
   already supports. If additional ESWIN-specific quirk bits are required, the
   PLIC driver may need small updates.

4. **DTB parsing failures**

   The real Megrez DTB may contain nodes or properties that the current
   `riscv_boot` code does not expect (for example CPU topology, cache
   information, or reserved memory).

## Current mitigation

- The boot assembly already trusts the DTB pointer in `a1` and falls back to a
  hard-coded DTB address if validation fails.
- UART markers in `ostd/src/arch/riscv/boot/boot.S` provide early feedback even
  before Rust code runs.

## Next steps

1. Confirm U-Boot can load the Asterinas ELF or a converted `booti` image at
   the expected address.
2. Observe the marker sequence on the Megrez serial console.
3. Fix the first failing milestone (MMU, trap handler, DTB parse, etc.).
