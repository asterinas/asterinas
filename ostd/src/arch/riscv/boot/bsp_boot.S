/* SPDX-License-Identifier: MPL-2.0 */

// The boot routine executed by the bootstrap processor (BSP) on RISC-V.

SATP_MODE_SV39         = 8 << 60
SATP_MODE_SV48         = 9 << 60
SATP_PPN_SHIFT         = 0

PTE_V                  = 0x01
PTE_R                  = 0x02
PTE_W                  = 0x04
PTE_X                  = 0x08
PTE_A                  = 0x40
PTE_D                  = 0x80
PTE_VRWXAD             = PTE_V | PTE_R | PTE_W | PTE_X | PTE_A | PTE_D
PTE_PPN_SHIFT          = 10
PTE_SIZE               = 8

PAGE_SHIFT             = 12

L2_PAGE_SHIFT          = 21

# The Sv39 boot page table identity-maps the first 128 GiB.
BOOT_IDENTITY_MAP_SIZE = 128 << 30

.section ".boot", "awx", @progbits

# Use PC-relative addressing until virtual memory and GP are initialized.
.option push
.option norelax

# The ELF entry is the default physical address of `_start`.
.global _start
_start:
    # Arguments passed from SBI:
    #   a0 = hart id
    #   a1 = device tree paddr
    # We do not touch them here. They are passed to the Rust entrypoint `riscv_boot`.

    # Discover the physical load address. Keep the existing Sv39 identity-map
    # limit (128 GiB) and require alignment for the 2 MiB kernel leaves.
    lla    t0, _start
    li     t1, (1 << L2_PAGE_SHIFT) - 1
    and    t1, t0, t1
    bnez   t1, bsp_unreachable_pa
    lla    t2, __kernel_end
    li     t1, BOOT_IDENTITY_MAP_SIZE
    bgtu   t2, t1, bsp_unreachable_pa

    # Unlike LLA, the literal contains the fixed linked virtual address.
    lla    t1, bsp_kernel_vaddr
    ld     t1, 0(t1)
    sub    t1, t1, t0
    lla    t3, __riscv_kernel_loaded_offset
    sd     t1, 0(t3)

    # Map the image at 0xffffffff80200000, starting at level-2 index 1.
    lla    t3, bsp_boot_l2pt
    addi   t3, t3, PTE_SIZE
    li     t4, 1 << L2_PAGE_SHIFT
bsp_map_kernel:
    srli   t1, t0, PAGE_SHIFT - PTE_PPN_SHIFT
    ori    t1, t1, PTE_VRWXAD
    sd     t1, 0(t3)
    addi   t3, t3, PTE_SIZE
    add    t0, t0, t4
    bltu   t0, t2, bsp_map_kernel

    # Both paging modes use the same level-2 kernel table.
    lla    t0, bsp_boot_l2pt
    srli   t0, t0, PAGE_SHIFT - PTE_PPN_SHIFT
    ori    t0, t0, PTE_V
    li     t2, 510 * PTE_SIZE
    lla    t1, sv39_boot_l3pt
    add    t1, t1, t2
    sd     t0, 0(t1)
    lla    t1, sv48_boot_l3pt
    add    t1, t1, t2
    sd     t0, 0(t1)

    # Set up the Sv48 page table.
    #   sv48_boot_l4pt[511] = (PPN(sv48_boot_l3pt) << PTE_PPN_SHIFT) | PTE_V
    lla    t1, sv48_boot_l4pt
    li     t0, 511 * PTE_SIZE
    add    t1, t1, t0
    lla    t0, sv48_boot_l3pt
    srli   t0, t0, PAGE_SHIFT - PTE_PPN_SHIFT
    ori    t0, t0, PTE_V
    sd     t0, 0(t1)

    # Make the newly written page tables visible to the page-table walker.
    sfence.vma

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
    lla    t0, sv39_boot_l3pt
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
    lla    t1, __riscv_kernel_loaded_offset
    ld     t1, 0(t1)
    lla    sp, boot_stack_top
    add    sp, sp, t1
    lla    t0, bsp_boot_virt
    add    t0, t0, t1
    jr     t0

.balign 8
bsp_kernel_vaddr:
    .quad __kernel_start

# Written once by the BSP before entering Rust; read by all harts afterwards.
.global __riscv_kernel_loaded_offset
__riscv_kernel_loaded_offset:
    .quad 0

.balign 4096
sv48_boot_l4pt:
    .quad (0x0 << PTE_PPN_SHIFT) | PTE_VRWXAD  # identity 0~512 GiB
    .zero 255 * PTE_SIZE
    .quad (0x0 << PTE_PPN_SHIFT) | PTE_VRWXAD  # linear 0~512 GiB
    .zero 254 * PTE_SIZE
    .quad 0                                    # TBA (-> boot_l3pt)
sv48_boot_l3pt:
    .zero 512 * PTE_SIZE  # Entry 510 is filled with bsp_boot_l2pt.

.balign 4096
sv39_boot_l3pt:
    .set i, 0
    .rept 128  # identity 0~128 GiB
        .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRWXAD
        .set i, i + 1
    .endr
    .zero 128 * PTE_SIZE
    .set i, 0
    .rept 128  # linear 0~128 GiB
        .quad ((i * 0x40000) << PTE_PPN_SHIFT) | PTE_VRWXAD
        .set i, i + 1
    .endr
    .zero 128 * PTE_SIZE  # Entry 510 is filled with bsp_boot_l2pt.

.balign 4096
bsp_boot_l2pt:
    .zero 512 * PTE_SIZE

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

.option pop

    # Jump into Rust code.
    lla    t0, riscv_boot
    jr     t0

bsp_unreachable_va:
    j      bsp_unreachable_va
