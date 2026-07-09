/* SPDX-License-Identifier: MPL-2.0 */

// The boot routine executed by the bootstrap processor (BSP) on AArch64.
//
// On entry (QEMU `virt`, Linux/EFI-stub-less `-kernel` boot protocol):
//   x0 = physical address of the Flattened Device Tree (DTB)
//   x1 = x2 = x3 = 0 (reserved)
//   The CPU may enter at EL2 or EL1 depending on the machine configuration.
//
// This routine drops to EL1 if necessary, installs a temporary set of boot page
// tables (identity map via TTBR0 + kernel high-half map via TTBR1), enables the
// MMU, switches to the kernel's virtual addresses, and jumps into the Rust
// entry point `aarch64_boot(dtb_paddr)`.

// ---- Descriptor / control-register constants -------------------------------

// Block/page lower attributes.
//   bit0      : valid
//   bit1      : 0 = block (L1/L2), 1 = table/page
//   bits[4:2] : AttrIndx (MAIR index)      0 = Normal WB, 1 = Device-nGnRnE
//   bits[7:6] : AP                          0b00 = EL1 RW, EL0 none
//   bits[9:8] : SH                          0b11 = inner shareable
//   bit10     : AF (access flag)
//   bit53     : PXN (privileged execute never)
//   bit54     : UXN (unprivileged execute never)
//
// Normal memory 1 GiB block, kernel RWX (UXN set, PXN clear):
//   (1<<54) | (1<<10) | (3<<8) | (0<<2) | 0b01 = 0x0040_0000_0000_0701
NORMAL_BLOCK = 0x0040000000000701
// Device memory 1 GiB block, no execute (PXN|UXN set):
//   (1<<54) | (1<<53) | (1<<10) | (0<<8) | (1<<2) | 0b01 = 0x0060_0000_0000_0405
DEVICE_BLOCK = 0x0060000000000405
// Table descriptor: next-level table address | 0b11
TABLE_DESC   = 0x3

.section ".boot", "awx", @progbits

.global _start
_start:
    // Preserve the DTB pointer across the whole boot sequence.
    mov     x19, x0

    // ---- Drop to EL1 if we entered at EL2 ---------------------------------
    mrs     x0, CurrentEL
    lsr     x0, x0, #2
    cmp     x0, #2
    b.ne    1f                       // already at EL1 (or EL0/EL3, treat as EL1)

    // Configure EL2 so that EL1 runs in AArch64 with no traps, then eret.
    mov     x0, #(1 << 31)           // HCR_EL2.RW = 1 (EL1 is AArch64)
    msr     hcr_el2, x0
    mrs     x0, midr_el1
    msr     vpidr_el2, x0
    mrs     x0, mpidr_el1
    msr     vmpidr_el2, x0
    mov     x0, #0x3c5               // DAIF masked, EL1h
    msr     spsr_el2, x0
    adr     x0, 1f
    msr     elr_el2, x0
    // Give EL1 its own stack selection.
    eret

1:
    // Use SP_EL1 (SPSel = 1) and mask interrupts during boot.
    msr     spsel, #1
    msr     daifset, #0xf

    // ---- Fill the single top-level (L0) table at runtime ------------------
    // Asterinas uses one root page table for the whole address space, with
    // TTBR0 translating the low (user) half and TTBR1 the high (kernel) half.
    // Both registers point at the same root, so a high-VA mapping added by the
    // boot page-table code (which walks TTBR0's root) is visible via TTBR1.
    //
    //   L0[0]   -> id_l1     : identity map (low VA, for the MMU-enable jump)
    //   L0[256] -> linear_l1 : linear map at 0xffff_8000_0000_0000
    //   L0[511] -> kcode_l1  : kernel code at 0xffff_ffff_0000_0000
    adrp    x2, boot_root_l0
    adrp    x1, id_l1
    orr     x1, x1, #TABLE_DESC
    str     x1, [x2, #0]
    adrp    x1, linear_l1
    orr     x1, x1, #TABLE_DESC
    str     x1, [x2, #(256 * 8)]
    adrp    x1, kcode_l1
    orr     x1, x1, #TABLE_DESC
    str     x1, [x2, #(511 * 8)]

    // ---- Program the translation control registers ------------------------
    // MAIR: Attr0 = Normal WB (0xFF), Attr1 = Device-nGnRnE (0x00).
    ldr     x0, =0x00000000000000FF
    msr     mair_el1, x0

    // TCR_EL1: T0SZ=T1SZ=16 (48-bit), 4 KiB granules, inner-shareable WBWA,
    // IPS = 48-bit. (Computed constant: 0x5_B510_3510)
    ldr     x0, =0x00000005B5103510
    msr     tcr_el1, x0

    adrp    x0, boot_root_l0
    msr     ttbr0_el1, x0
    msr     ttbr1_el1, x0

    // Ensure all the table writes are visible and invalidate stale TLBs.
    dsb     ish
    tlbi    vmalle1
    dsb     ish
    isb

    // ---- Enable the MMU ---------------------------------------------------
    mrs     x0, sctlr_el1
    orr     x0, x0, #(1 << 0)        // M: enable stage-1 MMU
    orr     x0, x0, #(1 << 2)        // C: data cache
    orr     x0, x0, #(1 << 12)       // I: instruction cache
    msr     sctlr_el1, x0
    isb

    // We are still executing at the identity-mapped physical PC (via TTBR0).
    // Jump to the kernel's high virtual address.
    ldr     x0, =bsp_boot_virt
    br      x0

// ---- Statically initialised block tables -----------------------------------
// linear_l1 backs the linear map at 0xffff_8000_0000_0000:
//   [0]   -> PA 0        (Device: covers GIC/UART/virtio-mmio in 0..1 GiB)
//   [1..] -> PA i GiB    (Normal: RAM)
.balign 4096
linear_l1:
    .quad DEVICE_BLOCK | (0 << 30)
    .set i, 1
    .rept 15
        .quad NORMAL_BLOCK | (i << 30)
        .set i, i + 1
    .endr
    .zero (512 - 16) * 8

// kcode_l1 backs the kernel code map at 0xffff_ffff_4020_0000:
//   [509] -> PA 0x4000_0000 (Normal, 1 GiB block covering the loaded kernel)
.balign 4096
kcode_l1:
    .zero 509 * 8
    .quad NORMAL_BLOCK | (0x40000000)
    .zero 2 * 8

// id_l1 backs the temporary identity map (TTBR0):
//   [0]   -> PA 0        (Device)
//   [1..] -> PA i GiB    (Normal)
.balign 4096
id_l1:
    .quad DEVICE_BLOCK | (0 << 30)
    .set i, 1
    .rept 15
        .quad NORMAL_BLOCK | (i << 30)
        .set i, i + 1
    .endr
    .zero (512 - 16) * 8

// The single top-level (L0) root table, populated at runtime and shared by
// TTBR0_EL1 and TTBR1_EL1.
.balign 4096
boot_root_l0:
    .zero 4096

.section ".boot.stack", "aw", @nobits
boot_stack_bottom:
    .balign 4096
    .skip 0x40000                    // 256 KiB
boot_stack_top:

// From here on we execute at kernel virtual addresses.
.text
bsp_boot_virt:
    // Set up the boot stack. `boot_stack_top` lives in the `.boot` section,
    // which is linked at low (physical) addresses, so OR in the kernel VMA
    // offset (0xffffffff00000000) to obtain its high-half alias. This alias is
    // mapped by both the boot page table and the kernel page table, so the
    // stack survives the switch to the kernel page table.
    ldr     x0, =boot_stack_top
    orr     x0, x0, #0xffffffff00000000
    mov     sp, x0

    // Initialise the CPU-local base register with the CPU-local storage start.
    ldr     x0, =__cpu_local_start
    msr     tpidr_el1, x0

    // Call into Rust with the DTB pointer as the first argument.
    mov     x0, x19
    ldr     x1, =aarch64_boot
    br      x1
