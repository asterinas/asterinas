# Issue 03: Asterinas hangs after `AEFGD` boot markers

## Symptom

After loading `aster-nix.booti` with U-Boot `booti` and starting the kernel, the
serial console prints:

```text
AEFGD
```

Then it hangs. The next expected marker `H` never appears.

## Context

The boot assembly in `ostd/src/arch/riscv/boot/boot.S` emits one character per
milestone:

| Marker | Milestone |
|--------|-----------|
| `A` | Entered `_start`, DTB validated |
| `E` | Early trap handler installed |
| `F` | Wrote `0` to `satp` |
| `G` | Read `satp` back |
| `D` | High-half page-table entry written |
| `H` | Sv48 SATP accepted and verified |
| `B` | MMU on, switching to high-half virtual addresses |
| `C` | Entered virtual-mode Rust entry `riscv_boot` |

The hang after `D` means the code failed somewhere between writing the page
table entry and verifying that `satp` was accepted.

## Cause

The non-leaf page-table entry that points from `boot_l4pt[511]` to
`boot_l3pt` was constructed with only `PTE_V`:

```asm
ori t0, t0, PTE_V
```

Some RISC-V hardware walkers require the **Accessed** bit (`PTE_A`) even on
non-leaf entries. Without it, the page-table walk can fault as soon as the MMU
is enabled, before the next instruction can be fetched.

## Fix

Set `PTE_A` on the non-leaf entry in `ostd/src/arch/riscv/boot/boot.S`:

```asm
ori t0, t0, PTE_V | PTE_A
```

Then rebuild:

```bash
export OSDK_LOCAL_DEV=1
cargo osdk build --scheme riscv --target-arch riscv64
```

## Verification

After the fix, the expected output should continue past `D`:

```text
AEFGDHB C...
```

> This fix has been applied and rebuilt; the new image is
> `/tmp/aster-nix-v3.booti`. It is waiting for the board to be physically reset
> so the test can run.
