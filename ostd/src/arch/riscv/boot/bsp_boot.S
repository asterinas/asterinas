/* SPDX-License-Identifier: MPL-2.0 */

// The boot routine executed by the bootstrap processor (BSP) on RISC-V.

SATP_MODE_SV39         = 8 << 60
SATP_MODE_SV48         = 9 << 60
SATP_PPN_SHIFT         = 0

PTE_V                  = 0x01
PTE_R                  = 0x02
PTE_W                  = 0x04
PTE_X                  = 0x08
PTE_PPN_SHIFT          = 10
PTE_SIZE               = 8

PAGE_SHIFT             = 12

KERNEL_VMA_OFFSET      = 0xffffffff00000000

.section ".boot", "awx", @progbits

# `_start` is also the entrypoint specified in the linker script.
.global _start
_start:
    # Arguments passed from SBI:
    #   a0 = hart id
    #   a1 = device tree paddr
    # We do not touch them here. They are passed to the Rust entrypoint `riscv_boot`.

    # Set up the Sv48 page table.
    #   sv48_boot_l4pt[511] = (PPN(sv48_boot_l3pt) << PTE_PPN_SHIFT) | PTE_V
    lla    t1, sv48_boot_l4pt
    li     t0, 511 * PTE_SIZE
    add    t1, t1, t0
    lla    t0, sv48_boot_l3pt
    srli   t0, t0, PAGE_SHIFT - PTE_PPN_SHIFT
    ori    t0, t0, PTE_V
    sd     t0, 0(t1)

    # Try loading the Sv48 page table.
    lla    t0, sv48_boot_l4pt
    li     t1, SATP_MODE_SV48
    srli   t0, t0, PAGE_SHIFT - SATP_PPN_SHIFT
    or     t0, t0, t1
    csrw   satp, t0

    # Check if the write to satp succeeds. If not, try Sv39.
    # Reference: <https://riscv.github.io/riscv-isa-manual/snapshot/privileged/#satp>.
    csrr   t1, satp
    beq    t0, t1, bsp_flush_tlb

    # Try loading the Sv39 page table.
    lla     t0, sv39_boot_l3pt
    li     t1, SATP_MODE_SV39
    srli   t0, t0, PAGE_SHIFT - SATP_PPN_SHIFT
    or     t0, t0, t1
    csrw   satp, t0

    # Check again if the write to satp succeeds.
    csrr   t1, satp
    beq    t0, t1, bsp_flush_tlb

    # If the CPU doesn't support either Sv48 or Sv39 this is actually reachable.
bsp_unreachable_pa:
    j      bsp_unreachable_pa

bsp_flush_tlb:
    sfence.vma

    # Update SP/PC to use the virtual address.
    li     t1, KERNEL_VMA_OFFSET
    lla    sp, boot_stack_top
    or     sp, sp, t1
    lla    t0, bsp_boot_virt - KERNEL_VMA_OFFSET
    or     t0, t0, t1
    jr     t0

PTE_VRWX = PTE_V | PTE_R | PTE_W | PTE_X

.balign 4096
sv48_boot_l4pt:
    .quad (0x0 << PTE_PPN_SHIFT) | PTE_VRWX  # identity 0~512 GiB
    .zero 255 * PTE_SIZE
    .quad (0x0 << PTE_PPN_SHIFT) | PTE_VRWX  # linear 0~512 GiB
    .zero 254 * PTE_SIZE
    .quad 0                                      # TBA (-> boot_l3pt)
sv48_boot_l3pt:  # 0xffff_ffff_0000_0000 -> 0x0000_0000_0000_0000
    .zero 508 * PTE_SIZE
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRWX  # code 0~1 GiB
    .quad (0x40000 << PTE_PPN_SHIFT) | PTE_VRWX  # code 1~2 GiB
    .quad (0x80000 << PTE_PPN_SHIFT) | PTE_VRWX  # code 2~3 GiB
    .quad 0

.balign 4096
sv39_boot_l3pt:
    .set i, 0
    .rept 128  # identity 0~128 GiB
        .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRWX
        .set i, i + 1
    .endr
    .zero 128 * PTE_SIZE
    .set i, 0
    .rept 128  # linear 0~128 GiB
        .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRWX
        .set i, i + 1
    .endr
    .zero 124 * PTE_SIZE
    .quad (0x00000 << PTE_PPN_SHIFT) | PTE_VRWX  # code 0~1 GiB
    .quad (0x40000 << PTE_PPN_SHIFT) | PTE_VRWX  # code 1~2 GiB
    .quad (0x80000 << PTE_PPN_SHIFT) | PTE_VRWX  # code 2~3 GiB
    .quad 0

.section ".boot.stack", "aw", @nobits

boot_stack_bottom:
    .balign 4096
    .skip 0x40000  # 256 KiB
boot_stack_top:

# From here, we no longer use physical address.

.text

bsp_boot_virt:
    # Initialize GP to the CPU-local storage's base address.
.extern __cpu_local_start
    lla    gp, __cpu_local_start

    # Jump into Rust code.
    lla    t0, riscv_boot
    jr     t0

bsp_unreachable_va:
    j      bsp_unreachable_va
