/* SPDX-License-Identifier: MPL-2.0 */

SATP_MODE_SV48         = 9 << 60
SATP_PPN_SHIFT         = 0

PTE_V                  = 0x01
PTE_R                  = 0x02
PTE_W                  = 0x04
PTE_X                  = 0x08
PTE_PPN_SHIFT          = 10
PTE_SIZE               = 8

PAGE_SHIFT             = 12

KERNEL_VMA             = 0xffffffff00000000

.section ".boot", "awx", @progbits
.globl _start
_start:
    # Arguments passed from SBI:
    #   a0 = hart id
    #   a1 = device tree paddr (not touched)

    # Set up the page table.
    #   boot_l4pt[511] = (PPN(boot_l3pt) << PTE_PPN_SHIFT) | PTE_V
    la     t1, boot_l4pt
    li     t0, 511 * PTE_SIZE
    add    t1, t1, t0
    la     t0, boot_l3pt
    srli   t0, t0, PAGE_SHIFT - PTE_PPN_SHIFT
    ori    t0, t0, PTE_V
    sd     t0, 0(t1)

    # Load the page table.
    la     t0, boot_l4pt
    li     t1, SATP_MODE_SV48
    srli   t0, t0, PAGE_SHIFT - SATP_PPN_SHIFT
    or     t0, t0, t1
    csrw   satp, t0
    sfence.vma

    # Update SP/PC to use the virtual address.
    li     t1, KERNEL_VMA
    la     sp, boot_stack_top
    or     sp, sp, t1
    la     t0, _start_virt - KERNEL_VMA
    or     t0, t0, t1
    jr     t0

PTE_VRWX = PTE_V | PTE_R | PTE_W | PTE_X

.balign 4096
boot_l4pt:
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRWX  # identity 0~512 GiB
    .zero 255 * PTE_SIZE
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRWX  # linear 0~512 GiB
    .zero 254 * PTE_SIZE
    .quad 0                                      # TBA (-> boot_l3pt)
boot_l3pt:  # 0xffff_ffff_0000_0000 -> 0x0000_0000_0000_0000
    .zero 508 * PTE_SIZE
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRWX  # code 0~1 GiB
    .quad (0x40000 << PTE_PPN_SHIFT) | PTE_VRWX  # code 1~2 GiB
    .quad (0x80000 << PTE_PPN_SHIFT) | PTE_VRWX  # code 2~3 GiB
    .quad 0

.section ".boot.stack", "aw", @nobits
.globl boot_stack_top
boot_stack_bottom:
    .balign 4096
    .skip 0x40000  # 256 KiB
boot_stack_top:

# From here, we're in the .text section: we no longer use physical address.
.text
.globl _start_virt
_start_virt:
    # Initialize GP to the CPU-local start address.
.extern __cpu_local_start
    la     gp, __cpu_local_start

    # Jump into Rust code.
    la     t0, riscv_boot
    jr     t0
