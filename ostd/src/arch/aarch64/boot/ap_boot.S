/* SPDX-License-Identifier: MPL-2.0 */

// The boot routine executed by application processors (APs) on AArch64.
//
// APs are started via the PSCI `CPU_ON` call, which enters at `ap_boot_start`
// (a physical address) with `x0` = the context ID we passed, which we use to
// carry the Asterinas CPU index.

NORMAL_BLOCK = 0x0040000000000701
DEVICE_BLOCK = 0x0060000000000405
TABLE_DESC   = 0x3

.section ".ap_boot", "awx", @progbits

.balign 4096
.global ap_boot_start
ap_boot_start:
    // x0 = context ID (the Asterinas CPU index). Preserve it.
    mov     x19, x0

    // Drop to EL1 if we entered at EL2.
    mrs     x0, CurrentEL
    lsr     x0, x0, #2
    cmp     x0, #2
    b.ne    1f
    mov     x0, #(1 << 31)              // HCR_EL2.RW = 1
    msr     hcr_el2, x0
    mov     x0, #0x3c5                  // EL1h, DAIF masked
    msr     spsr_el2, x0
    adr     x0, 1f
    msr     elr_el2, x0
    eret
1:
    msr     spsel, #1
    msr     daifset, #0xf

    // Load the shared boot page-table root (filled by the BSP). The pointer
    // variable lives in `.data` at a high virtual address, so compute its
    // physical location by subtracting the kernel VMA offset.
    ldr     x0, =__ap_boot_page_table_pointer
    movz    x1, #0xffff, lsl #48
    movk    x1, #0xffff, lsl #32        // x1 = 0xffffffff00000000
    sub     x0, x0, x1
    ldr     x0, [x0]                    // x0 = boot page-table root paddr

    // Program the translation registers (identical to the BSP).
    ldr     x2, =0x00000000000000FF
    msr     mair_el1, x2
    ldr     x2, =0x00000005B5103510
    msr     tcr_el1, x2
    msr     ttbr0_el1, x0
    msr     ttbr1_el1, x0

    dsb     ish
    tlbi    vmalle1
    dsb     ish
    isb

    mrs     x0, sctlr_el1
    orr     x0, x0, #(1 << 0)
    orr     x0, x0, #(1 << 2)
    orr     x0, x0, #(1 << 12)
    msr     sctlr_el1, x0
    isb

    ldr     x0, =ap_boot_virt
    br      x0

.data
.balign 8
// Filled by the BSP (`bringup_all_aps`) before starting APs.
.global __ap_boot_page_table_pointer
__ap_boot_page_table_pointer:
    .quad 0
.global __ap_boot_info_array_pointer
__ap_boot_info_array_pointer:
    .quad 0

.text
ap_boot_virt:
    // x19 = CPU index (context ID). Look up this AP's PerApRawInfo:
    //   info = __ap_boot_info_array_pointer; entry = &info[cpu_id - 1]
    //   PerApRawInfo is 16 bytes: stack_top at +0, cpu_local base at +8.
    ldr     x0, =__ap_boot_info_array_pointer
    ldr     x0, [x0]                    // x0 = &PerApRawInfo[0]
    sub     x1, x19, #1
    lsl     x1, x1, #4                  // (cpu_id - 1) * 16
    add     x0, x0, x1                  // x0 = &PerApRawInfo[cpu_id - 1]
    ldr     x2, [x0, #0]                // stack_top
    mov     sp, x2
    ldr     x2, [x0, #8]                // cpu_local base
    msr     tpidr_el1, x2

    mov     x29, #0                     // clear frame pointer

    // Call `ap_early_entry(cpu_id)`.
    mov     x0, x19
    ldr     x1, =ap_early_entry
    br      x1
