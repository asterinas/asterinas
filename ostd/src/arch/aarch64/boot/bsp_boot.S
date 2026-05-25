/* SPDX-License-Identifier: MPL-2.0 */

// The boot routine executed by the bootstrap processor (BSP) on ARM64.

.section ".boot", "awx", @progbits

// ARM64 Linux Image header (64 bytes)
// Reference: Documentation/arch/arm64/booting.rst in Linux kernel
// This header allows QEMU, U-Boot, GRUB, and other bootloaders to recognize
// Asterinas as a valid ARM64 kernel image.
.global _start
_start:
    // Offset 0-3: code0 - branch to actual_entry (skips header)
    // ARM64 instructions are always 4 bytes (u32)
    b       actual_entry                    // Jump past the 64-byte header

    // Offset 4-7: code1 - reserved for future use (u32)
    .long   0

    // Offset 8-15: text_offset - Image load offset (u64)
    // Standard value: 0x80000 (512KB). Bootloader places image at base + text_offset.
    .quad   0x80000                         // .quad = 8 bytes

    // Offset 16-23: image_size - Effective size of kernel image (u64)
    // 0 means bootloader should use actual file size
    .quad   0                               // .quad = 8 bytes

    // Offset 24-31: flags - Kernel flags (u64, little-endian)
    // Bit 0: endianness (0=LE, 1=BE)
    // Bits 1-2: page size (0=unspecified, 1=4K, 2=16K, 3=64K)
    // Asterinas: LE, 4K pages => flags = 0x00000002
    .quad   0x00000002                      // .quad = 8 bytes

    // Offset 32-39: res2 - reserved (u64, must be 0)
    .quad   0                               // .quad = 8 bytes

    // Offset 40-47: res3 - reserved (u64, must be 0)
    .quad   0                               // .quad = 8 bytes

    // Offset 48-55: res4 - reserved (u64, must be 0)
    .quad   0                               // .quad = 8 bytes

    // Offset 56-59: magic - ARM\x64 (u32, little-endian: 0x644d5241)
    // This is the KEY field that QEMU checks at ARM64_MAGIC_OFFSET (56)
    .ascii  "ARM\x64"                       // Exactly 4 bytes

    // Offset 60-63: res5 - PE/COFF offset (u32)
    // 0 = no EFI support
    .long   0                               // Must be .long (4 bytes)

actual_entry:
    // Arguments passed from bootloader (per ARM64 boot protocol):
    //   x0 = FDT physical address (mandatory for Linux boot)
    //   x1 = 0 (reserved)
    //   x2 = 0 (reserved)
    //   x3 = 0 (reserved)
    // We preserve x0 for the Rust entrypoint `aarch64_boot`.

    // Check current EL
    mrs     x1, CurrentEL
    and     x1, x1, #0xC           // Extract EL bits
    cmp     x1, #0x8               // EL2?
    b.eq    el2_setup

    // If not EL2, hang (we only support EL2 boot)
el_hang:
    wfi
    b       el_hang

el2_setup:
    // We boot at EL2 but the kernel runs at EL1. Configure EL2, then drop to EL1.

    // Set up HCR_EL2: set RW=1 (bit 31) to configure EL1 as AArch64.
    // Without this, eret to AArch64 EL1 is illegal because HCR_EL2.RW=0
    // means EL1 is AArch32. Linux sets this via init_el2_hcr (HCR_RW).
    ldr     x2, =0x80000000
    msr     hcr_el2, x2
    isb

    // Set up CPTR_EL2: don't trap FP/SIMD, SVE, SME, or TA accesses
    // Clear bits [31:20] to disable all traps
    mov     x2, #0
    msr     cptr_el2, x2
    isb

    // Set up CNTHCTL_EL2: enable EL1 physical timer access
    // EL1PCTEN=1 (allow EL1 to read physical counter)
    // EL1PCEN=1 (allow EL1 to access physical timer registers)
    mov     x2, #3
    msr     cnthctl_el2, x2
    isb

    // Set up ICC_SRE_EL2: enable System Register access for GICv3
    // SRE=1 (bit0), ENABLE=1 (bit3: allow EL1 to access ICC_* registers).
    // Without ENABLE, EL1 ICC_* access traps to EL2 (VBAR_EL2=0 → hang).
    // Linux: __init_el2_gicv3 sets SRE | ENABLE (ICC_SRE_EL2_SRE | ICC_SRE_EL2_ENABLE).
    mov     x2, #(1 << 3 | 1 << 0)    // 0x9: ENABLE | SRE
    msr     icc_sre_el2, x2
    isb

    // Now drop to EL1. The MMU is still off, so we can branch to el1_setup.
    // SPSR_EL2: M=5 (EL1h), D=1, A=1, I=1, F=1 (all interrupts masked) => 0x3C5
    adr     x2, el1_setup            // Return to el1_setup at EL1
    msr     elr_el2, x2
    ldr     x2, =0x3C5
    msr     spsr_el2, x2

    // Set SCTLR_EL1 to safe state (MMU and caches off) before dropping to EL1.
    // Firmware (UEFI) may have left SCTLR_EL1.I=1 with stale I-cache content,
    // which would cause instruction fetch faults at EL1. Linux writes
    // INIT_SCTLR_EL1_MMU_OFF before eret (head.S: init_el2 path).
    // clear M(0), C(2), I(12).
    mrs     x1, sctlr_el1
    mov     x2, #1
    bic     x1, x1, x2             // Disable MMU (bit 0)
    mov     x2, #4
    bic     x1, x1, x2             // Disable data cache (bit 2)
    mov     x2, #4096              // 1 << 12
    bic     x1, x1, x2             // Disable instruction cache (bit 12)
    msr     sctlr_el1, x1
    isb

    // Set SP for EL1 (identity mapped, use area below boot stack at PA)
    adrp    x2, boot_stack_top
    add     x2, x2, #:lo12:boot_stack_top
    mov     sp, x2

    eret                             // Transition to EL1, execution continues at el1_setup

el1_setup:
    // Disable MMU and caches before setting up page tables
    mrs     x1, sctlr_el1
    mov     x2, #1
    bic     x1, x1, x2             // Disable MMU (bit 0)
    mov     x2, #4
    bic     x1, x1, x2             // Disable data cache (bit 2)
    mov     x2, #4096              // 1 << 12
    bic     x1, x1, x2             // Disable instruction cache (bit 12)
    msr     sctlr_el1, x1
    isb

    // Set up TCR_EL1 for 48-bit VA with 4KB granule
    // T0SZ = 16 (48-bit VA for TTBR0)
    // TG0 = 0 (4KB granule for TTBR0: 0b00)
    // IRGN0 = 1, ORGN0 = 1 (normal cacheable)
    // SH0 = 3 (inner shareable)
    // T1SZ = 16 (48-bit VA for TTBR1)
    // TG1 = 2 (4KB granule for TTBR1: 0b10, ARM encodes TG1 differently from TG0)
    // IRGN1 = 1, ORGN1 = 1, SH1 = 3
    // Value: T0SZ=16 | T1SZ=16<<16 | SH0=3<<12 | SH1=3<<28 | IRGN0=1<<8 | IRGN1=1<<24
    //        | TG0=0 | TG1=2<<30 | IPS=5<<32 (40-bit PA)
    ldr     x1, =0x00000000b5103510
    msr     tcr_el1, x1
    isb

    // Set up MAIR_EL1 for memory attributes
    // Attr0: Device memory (0x00)
    // Attr1: Normal memory, non-cacheable (0x44)
    // Attr2: Normal memory, cacheable (0xff)
    ldr     x1, =0x00FF4400
    msr     mair_el1, x1
    isb

    // Load TTBR0_EL1 and TTBR1_EL1 with the same boot page table
    // This single page table contains both identity mapping and kernel VMA mapping
    adrp    x1, boot_l0_pt
    add     x1, x1, #:lo12:boot_l0_pt
    msr     ttbr0_el1, x1
    isb
    msr     ttbr1_el1, x1
    isb

    // Invalidate TLB
    tlbi    vmalle1is
    dsb     ish
    isb

    // Enable MMU and caches
    mrs     x1, sctlr_el1
    mov     x2, #1
    orr     x1, x1, x2             // Enable MMU (bit 0)
    mov     x2, #4
    orr     x1, x1, x2             // Enable data cache (bit 2)
    mov     x2, #4096              // 1 << 12
    orr     x1, x1, x2             // Enable instruction cache (bit 12)
    msr     sctlr_el1, x1
    isb

    // Now we're running with virtual addresses
    // Set up stack pointer (virtual address)
    // boot_stack_top is in .boot section at physical address, need to add offset
    adrp    x3, boot_stack_top
    add     x3, x3, #:lo12:boot_stack_top
    ldr     x4, =0xffffffff00000000
    add     sp, x3, x4

    // Jump to virtual address code
    // bsp_boot_virt is at virtual address, load directly
    ldr     x1, =bsp_boot_virt
    br      x1

.text

bsp_boot_virt:
    // Initialize tpidr_el1 to the CPU-local storage's base address.
    // __cpu_local_start is at virtual address
    .extern __cpu_local_start
    ldr     x5, =__cpu_local_start
    msr     tpidr_el1, x5

    // Jump into Rust code
    // x0 still contains FDT physical address
    // aarch64_boot is at virtual address
    ldr     x1, =aarch64_boot
    br      x1

// Boot page tables
// We use 1GB block mappings at L1 level
// These must be in .boot section so they are loaded at physical addresses

.section ".boot", "awx", @progbits
.balign 4096
boot_l0_pt:
    // Single L0 table for both TTBR0 and TTBR1
    // Entry 0: points to boot_l1_pt_identity for identity map (VA 0x0_0000_0000 - 0x0_FFFF_FFFF)
    .quad boot_l1_pt_identity + 0x3       // Table descriptor: valid + table bit
    // Entries 1-255: invalid
    .zero 255 * 8
    // Entry 256: points to boot_l1_pt_linear for linear mapping (VA 0xFFFF_8000_0000_0000 range)
    // LINEAR_MAPPING_BASE_VADDR = 0xffff_8000_0000_0000
    // Entry index = VA[47:39] = bits 47-39 of 0xFFFF_8000_0000_0000 = 256
    .quad boot_l1_pt_identity + 0x3       // Reuse identity L1 table for linear mapping
    // Entries 257-509: invalid
    // FRAME_METADATA range (L0 entry 448) is left absent; boot_pt::alloc_child
    // will allocate page table frames via early_alloc during meta::init().
    .zero 253 * 8
    // Entry 510: invalid (covers VA 0xFFFF_FF00_0000_0000 - 0xFFFF_FF7F_FFFF_FFFF)
    .quad 0
    // Entry 511: points to boot_l1_pt_kernel for kernel VMA (VA 0xFFFF_FF80_0000_0000 range)
    // Entry index = VA[47:39] = bits 47-39 of 0xFFFFFFFF_40000000 = 0x1FF = 511
    // This entry covers VA 0xFFFF_FF80_0000_0000 - 0xFFFF_FFFF_FFFF_FFFF
    .quad boot_l1_pt_kernel + 0x3         // Table descriptor: valid + table bit

.balign 4096
boot_l1_pt_identity:
    // L1 table for identity mapping (512 entries, each covers 1GB)
    // Entry 0: PA 0x00000000 -> VA 0x00000000 (device)
    // Covers UART at 0x09000000, GIC, etc.
    .quad 0x00000701
    // Entry 1: PA 0x40000000 -> VA 0x40000000 (normal)
    // Kernel RAM region - MUST use normal memory for code execution
    // New KERNEL_LMA: 0x40080000 (still within this 1GB block)
    .quad 0x40000709
    // Entry 2: PA 0x80000000 -> VA 0x80000000 (device)
    .quad 0x80000701
    // Entry 3: PA 0xC0000000 -> VA 0xC0000000 (device)
    .quad 0xC0000701
    // Remaining entries: invalid
    .zero 508 * 8

.balign 4096
boot_l1_pt_kernel:
    // L1 table for kernel high virtual addresses (512 entries, each covers 1GB)
    // New KERNEL_VMA: 0xffffffff40080000 -> Entry 509 covers this range
    // Entry 509: VA 0xffffffff40000000-0xffffffff7fffffff -> PA 0x40000000-0x7fffffff
    // Use 1GB blocks to cover: PA 0x40000000, 0x80000000, 0xC0000000
    .zero 509 * 8                         // Entries 0-508: invalid
    // Entry 509: VA 0xffffffff40000000 -> PA 0x40000000 (1GB block, normal)
    // Covers KERNEL_VMA at 0xffffffff40080000
    .quad 0x40000709
    // Entry 510: VA 0xffffffff80000000 -> PA 0x80000000 (1GB block, normal)
    .quad 0x80000709
    // Entry 511: VA 0xffffffffC0000000 -> PA 0xC0000000 (1GB block, normal)
    .quad 0xC0000709


.section ".boot.stack", "aw"

boot_stack_bottom:
    .balign 4096
    .fill 0x40000, 1, 0                   // 256 KiB zeroed stack
boot_stack_top: