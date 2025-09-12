/* SPDX-License-Identifier: MPL-2.0 */

// The boot routine executed by application processors (APs) on RISC-V.

SATP_MODE_SV48         = 9 << 60
SATP_PPN_SHIFT         = 0
PAGE_SHIFT             = 12
KERNEL_VMA             = 0xffffffff00000000

# This is to workaround <https://github.com/rust-lang/rust/issues/80608>.
.attribute arch, "rv64imac"

.section ".ap_boot", "awx", @progbits
.global ap_boot_start
.globl __ap_boot_info_array_pointer
.globl __ap_boot_page_table_pointer

.balign 4096
ap_boot_start:
    # At this point:
    #  - a0 contains the hart ID;
    #  - a1 is the opaque parameter (not used here);
    #  - We're running in M-mode or S-mode depending on SBI implementation.
    
    # Save hart ID in t4 for later use (t4 is caller-saved but we'll be careful).
    mv t4, a0
    
    # Load the page table address and set SATP.
    la t0, __ap_boot_page_table_pointer - KERNEL_VMA
    ld t1, 0(t0)                    # Load page table physical address.
    li t2, SATP_MODE_SV48           # SV48 mode.
    srli t1, t1, PAGE_SHIFT - SATP_PPN_SHIFT
    or t1, t1, t2
    csrw satp, t1
    sfence.vma                      # Flush TLB.
    
    # Now we need to switch to virtual addressing.
    # Calculate virtual address using the same method as boot.S.
    li t1, KERNEL_VMA
    la sp, ap_boot_virtual - KERNEL_VMA
    or sp, sp, t1
    jr sp

# From here, we're in the canonical virtual address space instead of linear
# physical addresses.

.data
.align 8
# These will be filled by the BSP before starting APs.
__ap_boot_page_table_pointer:
    .quad 0
__ap_boot_info_array_pointer:
    .quad 0

# This is atomically updated when a new hart is kicked on.
__ap_boot_cpu_id_tail:
    .quad 1

.text
ap_boot_virtual:
    # Atomically update the CPU ID tail and load the previous value to t1.
    li t1, 1
    la t2, __ap_boot_cpu_id_tail
    amoadd.w t1, t1, (t2)

    # Get the AP boot info array pointer using absolute addressing.
    la t2, __ap_boot_info_array_pointer
    ld t2, 0(t2)                    # Load pointer to `PerApRawInfo` array.

    # Each PerApRawInfo is 16 bytes (2 * 8-byte pointers).
    # Calculate offset: (cpu_id - 1) * 16.
    addi t3, t1, -1                 # `cpu_id - 1` (BSP is cpu 0, first AP is cpu 1).
    slli t3, t3, 4                  # Multiply by 16.
    add t3, t3, t2                  # `t3 = &per_ap_raw_info[cpu_id - 1]`

    # Load stack top and CPU local pointer.
    ld sp, 0(t3)                    # Load stack_top.
    ld t3, 8(t3)                    # Load cpu_local pointer.
    
    # Set up CPU-local storage in GP register.
    mv gp, t3
    
    # Clear frame pointer for clean stack traces.
    li fp, 0
    
    # Jump to Rust AP entry point using `la` like `boot.S`.
    # Pass CPU ID as first argument.
    mv a0, t1
    # Pass hardware hart ID as second argument.
    mv a1, t4
    la t1, riscv_ap_early_entry
    jr t1
    
    # Should never reach here
1:  wfi
    j 1b
