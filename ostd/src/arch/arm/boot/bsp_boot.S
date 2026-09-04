/* SPDX-License-Identifier: MPL-2.0 */

// The boot routine executed by the bootstrap processor (BSP) on ARM.

KERNEL_VMA             = 0xffffffff00000000

# The base address of the DRAM.
#
# This is necessary because the AArch64 Linux boot protocol only allows the
# text offset to be specified relative to the DRAM base. We may want to figure
# out a better way since this constant is likely specific to QEMU.
DRAM_BASE              = 0x40000000

# Translation Control Register (TCR)
#
# Bits separated for:    [  TTBR1  ]   [  TTBR0  ]
TCR_TG_4K              = ( 2 << 30 ) | ( 0 << 14 )  // Page size: 4 KiB
TCR_SH_INNER           = ( 3 << 28 ) | ( 3 << 12 )  // Shareability: Inner sharable
TCR_ORGN_WBWA          = ( 1 << 26 ) | ( 1 << 10 )  // Outer cacheability: Write-back write-allocate
TCR_IRGN_WBWA          = ( 1 << 24 ) | ( 1 <<  8 )  // Inner cacheability: Write-back write-allocate
TCR_TSZ_48             = (16 << 16 ) | (16 <<  0 )  // Virtual address width: 48 bits
# Bits for both tables:
TCR_IPS_48             =   5 << 32                  // Physical address width: 48 bits

# Memory Attribute Indirection Register (MAIR)
MAIR_0WBWA             = (0xFF << 0)  // Attr0: Normal memory, write-back, write-allocate
MAIR_1DEVICE           = (0x00 << 8)  // Attr1: Device-nGnRnE memory

# System Control Register (SCTLR)
SCTLR_I                = 1 << 12      // Instruction access cacheability: Cacheable
SCTLR_C                = 1 <<  2      // Data access cacheability: Cacheable
SCTLR_M                = 1 <<  0      // MMU state: Enabled

# PTE flags
PTE_VALID              = 1 <<  0      // Valid
PTE_NON_HUGE           = 1 <<  1      // Non-huge: Middle-level non-leaf or last-level leaf
PTE_SH_INNER           = 3 <<  8      // Shareability: Inner sharable
PTE_ACCESS             = 1 << 10      // Access: Accessed

# PTE size
PTE_SIZE               = 8

.section ".boot", "awx", @progbits

# AArch64 Linux Boot Protocol
# Reference: <https://www.kernel.org/doc/html/v7.1/arch/arm64/booting.html#call-the-kernel-image>
_header:
    bl      _start               // Executable code
_header_4:
    .long   0                    // Executable code
    .quad   _header - DRAM_BASE  // Image load offset, little endian
.extern __kernel_size
    .quad   __kernel_size        // Effective image size, little endian
    .quad   0b0010               // Kernel flags, little endian
                                 //   Endianness: LE (Bit 0, value 0)
                                 //   Page size: 4K (Bit 1-2, value 1)
                                 //   Physical placement: close to the DRAM base (Bit 3, value 0)
    .quad   0                    // Reserved
    .quad   0                    // Reserved
    .quad   0                    // Reserved
    .long   0x644d5241           // Magic number, little endian, "ARM\x64"
    .long   0                    // Reserved

# `_start` is also the entrypoint specified in the linker script.
.global _start
_start:
    # Arguments passed from the boot loader:
    #   x0 = physical address of device tree blob (dtb) in system RAM
    #   x1 = 0 (reserved for future use)
    #   x2 = 0 (reserved for future use)
    #   x3 = 0 (reserved for future use)
    # We do not touch them here. They are passed to the Rust entrypoint `arm_boot`.

    # Die if the kernel is loaded at a wrong address.
    # We need to relocate, which is not supported.
    ldr     x4, =_header_4
    cmp     x4, x30
die_load_address:
    b.ne    die_load_address

    # Die if the kernel is executed at a wrong exception level.
    # We do not support exception levels other than EL1.
    mrs     x4, currentel
    cmp     x4, #(0b01 << 2)  // EL, bits [3:2]: 01 = EL1
die_exception_level:
    b.ne    die_exception_level

    # Set up translation control.
    ldr     x4, =(TCR_TG_4K | TCR_SH_INNER | TCR_ORGN_WBWA | TCR_IRGN_WBWA | TCR_TSZ_48 | TCR_IPS_48)
    msr     tcr_el1, x4

    # Set up memory attributes.
    mov     x4, #(MAIR_0WBWA | MAIR_1DEVICE)
    msr     mair_el1, x4

    # Load translation tables.
    ldr     x4, =boot_l4pt
    msr     ttbr0_el1, x4
    msr     ttbr1_el1, x4

    # Synchronize the new translation context.
    isb

    # Flush TLBs before enabling MMU.
    tlbi    vmalle1
    dsb     nsh

    # Enable MMU and cache.
    mov     x4, #(SCTLR_I | SCTLR_C | SCTLR_M)
    msr     sctlr_el1, x4
    isb

    # Update SP/PC to use the virtual address.
    ldr     x4, =(boot_stack_top + KERNEL_VMA)
    mov     sp, x4
    ldr     x4, =_start_virt
    br      x4

.balign 4096
boot_l4pt:
    .quad boot_l3pt_linear_id + PTE_VALID + PTE_NON_HUGE  // identity
    .zero 255 * PTE_SIZE
    .quad boot_l3pt_linear_id + PTE_VALID + PTE_NON_HUGE  // linear
    .zero 254 * PTE_SIZE
    .quad boot_l3pt_kernel    + PTE_VALID + PTE_NON_HUGE  // code
boot_l3pt_linear_id:  # 0xffff_8000_0000_0000 -> 0x0000_0000_0000_0000
    .set i, 0
    .rept 512  // linear 0~512 GiB
        .quad (i << 30) | PTE_VALID | PTE_SH_INNER | PTE_ACCESS
        .set i, i + 1
    .endr
boot_l3pt_kernel:  # 0xffff_ffff_0000_0000 -> 0x0000_0000_0000_0000
    .zero 508 * PTE_SIZE
    .quad (0 << 30) | PTE_VALID | PTE_SH_INNER | PTE_ACCESS  // code 0~1 GiB
    .quad (1 << 30) | PTE_VALID | PTE_SH_INNER | PTE_ACCESS  // code 1~2 GiB
    .quad (2 << 30) | PTE_VALID | PTE_SH_INNER | PTE_ACCESS  // code 2~3 GiB
    .quad 0

.section ".boot.stack", "awx", @nobits

boot_stack_bottom:
    .balign 4096
    .skip 0x40000
boot_stack_top:

# From here, we no longer use physical address.

.text

_start_virt:
    # Initialize TPIDR to the CPU-local storage's base address.
.extern __cpu_local_start
    ldr     x4, =__cpu_local_start
    msr     tpidr_el1, x4

    # Jump into Rust code.
    b       arm_boot
