/* SPDX-License-Identifier: MPL-2.0 */

// The boot routine executed by application processors (APs) on RISC-V.

SATP_MODE_SV39         = 8 << 60
SATP_MODE_SV48         = 9 << 60
SATP_PPN_SHIFT         = 0
PAGE_SHIFT             = 12
KERNEL_VMA_OFFSET      = 0xffffffff00000000

# This is to workaround <https://github.com/rust-lang/rust/issues/80608>.
.attribute arch, "rv64imac"

.section ".ap_boot", "awx", @progbits

.balign 4096
.global ap_boot_start
ap_boot_start:
    # At this point:
    #  - a0 contains the hart ID;
    #  - a1 is the opaque parameter (not used here);
    #  - We're running in M-mode or S-mode depending on SBI implementation.
    
    # Save hart ID in t4 for later use (t4 is caller-saved but we'll be careful).
    mv      t4, a0
    
    # Load the page table address in Sv48 mode and set SATP.
    lla     t0, __ap_boot_page_table_pointer - KERNEL_VMA_OFFSET
    ld      t1, 0(t0)                   # Load page table physical address.
    li      t2, SATP_MODE_SV48          # Sv48 mode.
    srli    t1, t1, PAGE_SHIFT - SATP_PPN_SHIFT
    or      t1, t1, t2
    csrw    satp, t1

    # Check if the write to satp succeeds.
    # Reference: <https://riscv.github.io/riscv-isa-manual/snapshot/privileged/#satp>.
    csrr    t3, satp
    beq     t3, t1, flush_tlb_ap

    # This AP doesn't support Sv48. So the `__ap_boot_page_table_pointer` must
    # point to a Sv39 page table since we assume that all harts support a same
    # paging mode.
    xor     t1, t1, t2                  # Clear previous mode bits.
    li      t2, SATP_MODE_SV39          # Sv39 mode.
    or      t1, t1, t2
    csrw    satp, t1

    # Check again if the write to satp succeeds.
    csrr    t0, satp
    beq     t0, t1, flush_tlb_ap

    # If the CPU doesn't support either Sv48 or Sv39 this is actually reachable.
unreachable_pa_ap:
    j      unreachable_pa_ap

flush_tlb_ap:
    sfence.vma                          # Flush TLB.
    
    # Now we need to switch to virtual addressing.
    # Calculate virtual address using the same method as boot.S.
    li      t1, KERNEL_VMA_OFFSET
    lla     sp, ap_boot_virtual - KERNEL_VMA_OFFSET
    or      sp, sp, t1
    jr      sp

# From here, we're in the canonical virtual address space instead of linear
# physical addresses.

.data

.balign 8
# These will be filled by the BSP before starting APs.
.global __ap_boot_page_table_pointer
__ap_boot_page_table_pointer:
    .quad 0
.global __ap_boot_info_array_pointer
__ap_boot_info_array_pointer:
    .quad 0

# This is atomically incremented when a new hart is kicked on. The BSP's ID is
# 0, and the first AP's ID will be 1. 
__ap_boot_cpu_id_tail:
    .quad 1

.text

ap_boot_virtual:
    # Atomically update the CPU ID tail and load the previous value to t1.
    lla     t0, __ap_boot_cpu_id_tail
cmpxchg_load:
    lr.d    t1, (t0)
    addi    t2, t1, 1
    sc.d    t2, t2, (t0)
    bnez    t2, cmpxchg_load

    # Get the AP boot info array pointer using absolute addressing.
    lla     t2, __ap_boot_info_array_pointer
    ld      t2, 0(t2)                   # Load pointer to `PerApRawInfo` array.

    # Each PerApRawInfo is 16 bytes (2 * 8-byte pointers).
    # Calculate offset: (cpu_id - 1) * 16.
    addi    t3, t1, -1                  # `cpu_id - 1` (BSP is cpu 0, first AP is cpu 1).
    slli    t3, t3, 4                   # Multiply by 16.
    add     t3, t3, t2                  # `t3 = &per_ap_raw_info[cpu_id - 1]`

    # Load stack top and CPU local storage's base address.
    ld      sp, 0(t3)                   # Load stack_top.
    ld      gp, 8(t3)                   # Load cpu_local pointer.
    
    # Clear frame pointer for clean stack traces.
    li      fp, 0
    
    # Jump to Rust AP entry point.
    # Pass CPU ID as the first argument.
    mv      a0, t1
    # Pass the hardware hart ID as the second argument.
    mv      a1, t4
    lla     t1, riscv_ap_early_entry
    jr      t1

unreachable_va_ap:
    j      unreachable_va_ap
