/* SPDX-License-Identifier: MPL-2.0 */

// Minimal self-contained RISC-V Sv39 boot test for QEMU virt.
// Kernel is linked at 0xffffffff80200000, loaded at 0x80200000.

SATP_MODE_SV39 = 8 << 60

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
    li t3, 0x10000000
    li t4, \c
    sb t4, 0(t3)
.endm

.global _start
_start:
    UART_CH 'A'

    lla    t0, _early_trap
    csrw   stvec, t0
    UART_CH 'E'

    csrw   satp, zero
    UART_CH 'F'
    csrr   t1, satp
    UART_CH 'G'

    UART_CH 'D'

    sfence.vma

    lla    t0, boot_l3pt
    li     t1, SATP_MODE_SV39
    srli   t0, t0, PAGE_SHIFT
    or     t0, t0, t1
    csrw   satp, t0
    sfence.vma

    csrr   t1, satp
    UART_CH 'H'
    bne    t0, t1, .Lmmu_hang

    UART_CH 'B'

    li     t1, KERNEL_VMA_OFFSET

    lla    sp, boot_stack_top
    or     sp, sp, t1

    lla    gp, __cpu_local_start
    or     gp, gp, t1

    lla    t0, riscv_boot
    or     t0, t0, t1
    jr     t0

.Lmmu_hang:
    UART_CH '!'
1:  j      1b

.balign 4
_early_trap:
    csrr   t0, scause
    andi   t0, t0, 0x7f
    li     t3, 0x10000000
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

.balign 4096
boot_l3pt:
    .set i, 0
    .rept 128
    .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRXWAD
    .set i, i + 1
    .endr
    .zero 128 * PTE_SIZE
    .set i, 0
    .rept 128
    .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRXWAD
    .set i, i + 1
    .endr
    // High-half mapping for 0xffffffff80200000 (root index 510 -> phys 0).
    .zero 124 * PTE_SIZE
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRXWAD
    .quad (0x40000 << PTE_PPN_SHIFT) | PTE_VRXWAD
    .quad (0x80000 << PTE_PPN_SHIFT) | PTE_VRXWAD
    .quad (0xc0000 << PTE_PPN_SHIFT) | PTE_VRXWAD

.section ".bss.stack", "aw", @nobits

.balign 4096
boot_stack_bottom:
    .skip 0x40000
boot_stack_top:

.text

.global __cpu_local_start
__cpu_local_start:
    .quad 0

.global riscv_boot
riscv_boot:
    UART_CH 'C'
1:
    wfi
    j 1b
