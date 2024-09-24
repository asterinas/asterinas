/* SPDX-License-Identifier: MPL-2.0 */

.section .text.entry
.globl _start
_start:
    # Arguments passed from SBI:
    #   a0 = hart id
    #   a1 = device tree paddr (not touched)

    # 1. enable paging
    # setting up 1st pagetable
    #   entry = (PPN(boot_pagetable_2nd) << 10) | 0x01 # V
    la     t1, boot_pagetable
    li     t0, 8 * 511
    add    t1, t1, t0
    la     t0, boot_pagetable_2nd
    srli   t0, t0, 2
    ori    t0, t0, 0x01
    sd     t0, 0(t1)

    la     t0, boot_pagetable
    li     t1, 9 << 60
    srli   t0, t0, 12
    or     t0, t0, t1
    csrw   satp, t0
    sfence.vma

    # 2. set sp (BSP only)
    lga    sp, boot_stack_top

    # 3. jump to rust riscv_boot
    lga    t0, riscv_boot
    jr     t0


.section .bss.stack

.globl boot_stack_bottom
boot_stack_bottom:
    .space 0x40000 # 64 KiB

.globl boot_stack_top
boot_stack_top:


.section .data

.align 12
boot_pagetable:
    .quad (0x00000 << 10) | 0xcf # VRWXAD
    .zero 8 * 255
    .quad (0x00000 << 10) | 0xcf # VRWXAD
    .zero 8 * 254
    .quad 0  # To-Be-Assign

boot_pagetable_2nd:
    # 0x0000_00ff_8000_0000 -> 0x0000_0000_8000_0000
    .zero 8 * 508
    .quad (0x00000 << 10) | 0xcf # VRWXAD
    .quad (0x40000 << 10) | 0xcf # VRWXAD
    .quad (0x80000 << 10) | 0xcf # VRWXAD
    .quad 0
