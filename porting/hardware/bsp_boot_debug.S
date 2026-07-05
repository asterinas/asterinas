/* SPDX-License-Identifier: MPL-2.0 */

// The boot routine executed by the bootstrap processor (BSP) on RISC-V.
//
// Debug build for Milk-V Megrez / EIC7700:
//   - trusts a0 (hartid) and a1 (dtb) from U-Boot booti (no mhartid read)
//   - falls back to a hard-coded dtb at 0xf0000000 if a1 is not a valid FDT
//   - adds A/D bits to all leaf page-table entries (real hardware needs them)
//   - adds MMIO UART markers and a tiny trap handler for early debugging

SATP_MODE_SV39 = 8 << 60
SATP_MODE_SV48 = 9 << 60
SATP_PPN_SHIFT = 0

PTE_V = 0x01
PTE_R = 0x02
PTE_W = 0x04
PTE_X = 0x08
PTE_A = 0x40
PTE_D = 0x80
PTE_PPN_SHIFT = 10
PTE_SIZE = 8

PAGE_SHIFT = 12

KERNEL_VMA_OFFSET = 0xffffffff00000000

.section ".boot", "awx", @progbits

.macro UART_CH c
    li t3, 0x50900000
    li t4, \c
    sb t4, 0(t3)
.endm

.global _start
_start:
    # a0 = hart id, a1 = dtb paddr from U-Boot booti / SBI.
    # Do not read mhartid here: EIC7700 enters in S-mode and that traps.

    li     t0, 0xedfe0dd0          # FDT magic on little-endian
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

    # Install a tiny physical-mode trap handler.
    la t0, _early_trap
    csrw stvec, t0
    UART_CH 'E'

    # Sanity-check that satp is accessible.
    csrw satp, zero
    UART_CH 'F'
    csrr t1, satp
    UART_CH 'G'

    # sv48_boot_l4pt[511] = PPN(sv48_boot_l3pt) | V | A
    lla t1, sv48_boot_l4pt
    li t0, 511 * PTE_SIZE
    add t1, t1, t0
    lla t0, sv48_boot_l3pt
    srli t0, t0, PAGE_SHIFT - PTE_PPN_SHIFT
    ori t0, t0, PTE_V | PTE_A
    sd t0, 0(t1)

    UART_CH 'D'

    # Make the page-table writes visible to the hardware walker before
    # enabling paging.  Without this the first translated fetch may see
    # stale entries and fault before the trap handler can run.
    sfence.vma

    # Try Sv48.
    lla t0, sv48_boot_l4pt
    li t1, SATP_MODE_SV48
    srli t0, t0, PAGE_SHIFT - SATP_PPN_SHIFT
    or t0, t0, t1
    csrw satp, t0
    sfence.vma

    csrr t1, satp
    UART_CH 'H'
    beq t0, t1, bsp_flush_tlb

    # Try Sv39 fallback.
    lla t0, sv39_boot_l3pt
    li t1, SATP_MODE_SV39
    srli t0, t0, PAGE_SHIFT - SATP_PPN_SHIFT
    or t0, t0, t1
    csrw satp, t0
    sfence.vma

    csrr t1, satp
    UART_CH 'h'
    beq t0, t1, bsp_flush_tlb

.Ldtb_hang:
.Lsatp_hang:
bsp_unreachable_pa:
    UART_CH '!'
    j bsp_unreachable_pa

bsp_flush_tlb:
    sfence.vma
    UART_CH 'B'

    li t1, KERNEL_VMA_OFFSET
    lla sp, boot_stack_top
    or sp, sp, t1
    lla t0, bsp_boot_virt - KERNEL_VMA_OFFSET
    or t0, t0, t1
    jr t0

PTE_VRXWAD = PTE_V | PTE_R | PTE_W | PTE_X | PTE_A | PTE_D

.balign 4096
sv48_boot_l4pt:
    .quad (0x0 << PTE_PPN_SHIFT) | PTE_VRXWAD # identity 0~512 GiB
    .zero 255 * PTE_SIZE
    .quad (0x0 << PTE_PPN_SHIFT) | PTE_VRXWAD # linear 0~512 GiB
    .zero 254 * PTE_SIZE
    .quad 0
sv48_boot_l3pt:
    .zero 508 * PTE_SIZE
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRXWAD # code 0~1 GiB
    .quad (0x40000 << PTE_PPN_SHIFT) | PTE_VRXWAD # code 1~2 GiB
    .quad (0x80000 << PTE_PPN_SHIFT) | PTE_VRXWAD # code 2~3 GiB
    .quad 0

.balign 4096
sv39_boot_l3pt:
    .set i, 0
    .rept 128 # identity 0~128 GiB
    .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRXWAD
    .set i, i + 1
    .endr
    .zero 128 * PTE_SIZE
    .set i, 0
    .rept 128 # linear 0~128 GiB
    .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRXWAD
    .set i, i + 1
    .endr
    .zero 124 * PTE_SIZE
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRXWAD # code 0~1 GiB
    .quad (0x40000 << PTE_PPN_SHIFT) | PTE_VRXWAD # code 1~2 GiB
    .quad (0x80000 << PTE_PPN_SHIFT) | PTE_VRXWAD # code 2~3 GiB
    .quad 0

.section ".boot.stack", "aw", @nobits

boot_stack_bottom:
    .balign 4096
    .skip 0x40000
boot_stack_top:

.text

bsp_boot_virt:
    UART_CH 'C'

    .extern __cpu_local_start
    lla gp, __cpu_local_start

    lla t0, riscv_boot
    jr t0

bsp_unreachable_va:
    j bsp_unreachable_va

.balign 4
_early_trap:
    csrr t0, scause
    andi t0, t0, 0x7f
    li t3, 0x50900000
    li t4, 'T'
    sb t4, 0(t3)
    andi t4, t0, 0xf
    li t5, 9
    ble t4, t5, 1f
    addi t4, t4, 'a' - 10
    j 2f
1:
    addi t4, t4, '0'
2:
    sb t4, 0(t3)
3:
    j 3b
