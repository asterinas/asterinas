/* SPDX-License-Identifier: MPL-2.0 */

// RISC-V BSP entrypoint for Milk-V Megrez / EIC7700.
//
// Assumptions:
//   - Entered in S-mode from U-Boot `booti` with a0=hartid, a1=DTB paddr.
//   - Do NOT read mhartid: it traps on EIC7700 in S-mode.
//   - U-Boot has initialised the UART at 0x50900000; we can write THR directly.
//   - The kernel is loaded at physical 0x80200000 and linked at 0xffffffff80200000.

SATP_MODE_SV39 = 8 << 60
SATP_MODE_SV48 = 9 << 60

PTE_V  = 0x01
PTE_R  = 0x02
PTE_W  = 0x04
PTE_X  = 0x08
PTE_A  = 0x40
PTE_D  = 0x80

PTE_PPN_SHIFT = 10
PTE_SIZE      = 8
PAGE_SHIFT    = 12

KERNEL_VMA_OFFSET = 0xffffffff00000000

.section ".text.entry", "ax", @progbits

.macro UART_CH c
    li t3, 0x50900000
    li t4, \c
    sb t4, 0(t3)
.endm

.global _start
_start:
    // a0 = hart id, a1 = DTB paddr (from U-Boot / SBI).

    // Validate DTB pointer; fall back to the address we usually load it at.
    li     t0, 0xedfe0dd0          // FDT magic (little-endian)
    li     t2, 0x80000000
    bltu   a1, t2, .Luse_default_dtb
    lwu    t1, 0(a1)
    beq    t0, t1, .Ldtb_ok
.Luse_default_dtb:
    li     a1, 0xf0000000
    lwu    t1, 0(a1)
    bne    t0, t1, .Ldtb_hang
.Ldtb_ok:
    UART_CH 'A'

    // Install a tiny physical-mode trap handler so we can see scause if
    // something faults before the kernel installs its own stvec.
    lla    t0, _early_trap
    csrw   stvec, t0
    UART_CH 'E'

    // Sanity-check that satp is accessible in S-mode.
    csrw   satp, zero
    UART_CH 'F'
    csrr   t1, satp
    UART_CH 'G'

    // --- Build the Sv48 page table ------------------------------------------
    // L4[0]   -> 512 GiB identity leaf mapping physical 0
    // L4[256] -> 512 GiB linear leaf mapping physical 0
    // L4[511] -> boot_l3pt (high-half mapping for the kernel VMA)
    lla    t1, boot_l4pt

    lla    t0, boot_l3pt
    srli   t0, t0, PAGE_SHIFT - PTE_PPN_SHIFT
    ori    t0, t0, PTE_V          // non-leaf PTE: only V (A/D/R/W/X reserved)
    li     t2, 511 * PTE_SIZE
    add    t1, t1, t2
    sd     t0, 0(t1)               // L4[511]

    UART_CH 'D'

    // Synchronise page-table writes before turning on the MMU.
    sfence.vma

    // Try Sv48.
    lla    t0, boot_l4pt
    li     t1, SATP_MODE_SV48
    srli   t0, t0, PAGE_SHIFT
    or     t0, t0, t1
    csrw   satp, t0
    sfence.vma

    csrr   t1, satp
    UART_CH 'H'
    beq    t0, t1, .Lmmu_ok

    // Try Sv39 fallback.
    lla    t0, boot_l3pt_sv39
    li     t1, SATP_MODE_SV39
    srli   t0, t0, PAGE_SHIFT
    or     t0, t0, t1
    csrw   satp, t0
    sfence.vma

    csrr   t1, satp
    UART_CH 'h'
    beq    t0, t1, .Lmmu_ok

.Ldtb_hang:
.Lmmu_hang:
    UART_CH '!'
1:  j      1b

.Lmmu_ok:
    UART_CH 'B'

    // Switch SP/PC to the high-half virtual addresses.
    li     t1, KERNEL_VMA_OFFSET

    lla    sp, boot_stack_top
    or     sp, sp, t1

    lla    gp, __cpu_local_start
    or     gp, gp, t1

    lla    t0, riscv_boot
    or     t0, t0, t1
    jr     t0

// Tiny trap handler: prints 'T' followed by the low nibble of scause.
.balign 4
_early_trap:
    csrr   t0, scause
    andi   t0, t0, 0x7f
    li     t3, 0x50900000
    li     t4, 'T'
    sb     t4, 0(t3)
    andi   t4, t0, 0xf
    li     t5, 9
    ble    t4, t5, 1f
    addi   t4, t4, 'a' - 10
    j      2f
1:
    addi   t4, t4, '0'
2:
    sb     t4, 0(t3)
3:
    j      3b

PTE_VRXWAD = PTE_V | PTE_R | PTE_W | PTE_X | PTE_A | PTE_D

// ---- Sv48 page tables ------------------------------------------------------
.balign 4096
boot_l4pt:
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // L4[0]: identity 0-512 GiB
    .zero 255 * PTE_SIZE
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // L4[256]: linear 0-512 GiB
    .zero 254 * PTE_SIZE
    .quad 0                                          // L4[511]: -> boot_l3pt

boot_l3pt:
    // For KERNEL_VMA = 0xffffffff80200000 under Sv48:
    //   VPN3 = 511, VPN2 = 510 -> region base 0xffffffff80000000.
    //   Map that 1 GiB region to physical 0x80000000 (2 GiB) so that
    //   virtual 0xffffffff80200000 resolves to physical 0x80200000.
    .zero 508 * PTE_SIZE
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // 0xffffffff00000000 -> 0 GiB
    .quad (0x40000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // 0xffffffff40000000 -> 1 GiB
    .quad (0x80000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // 0xffffffff80000000 -> 2 GiB (kernel)
    .quad (0xc0000 << PTE_PPN_SHIFT) | PTE_VRXWAD  // 0xffffffffc0000000 -> 3 GiB

// ---- Sv39 fallback page table ----------------------------------------------
.balign 4096
boot_l3pt_sv39:
    // Identity mapping 0-128 GiB.
    .set i, 0
    .rept 128
    .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRXWAD
    .set i, i + 1
    .endr
    .zero 128 * PTE_SIZE
    // Linear 0-128 GiB.
    .set i, 0
    .rept 128
    .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRXWAD
    .set i, i + 1
    .endr
    // High-half mapping for 0xffffffff80200000 (Sv39 root index 510 -> phys 2 GiB).
    .zero 124 * PTE_SIZE
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRXWAD
    .quad (0x40000 << PTE_PPN_SHIFT) | PTE_VRXWAD
    .quad (0x80000 << PTE_PPN_SHIFT) | PTE_VRXWAD
    .quad (0xc0000 << PTE_PPN_SHIFT) | PTE_VRXWAD

// ---- Boot stack ------------------------------------------------------------
.section ".bss.stack", "aw", @nobits

.balign 4096
boot_stack_bottom:
    .skip 0x40000  // 256 KiB
boot_stack_top:
