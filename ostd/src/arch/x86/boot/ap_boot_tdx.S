/* SPDX-License-Identifier: MPL-2.0 */

// The boot routine executed by the application processor in Intel TDX environment.

.extern boot_gdtr
.extern ap_early_entry

.section ".ap_boot", "awx"
.align 4096
.code64

// This is a pointer to the page table used by the APs.
// The BSP will fill this pointer before kicking the APs.
.global __boot_page_table_pointer
.align 8
__boot_page_table_pointer:
    .skip 8

ap_long_mode_in_low_address:
    cli
    cld

    mov ax, 0
    mov ds, ax
    mov ss, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    // The Local APIC ID information is stored in r8d by Intel TDX Virtual Firmware.
    mov edi, r8d

    lgdt [boot_gdtr]

    // Enable PAE and PGE.
    mov rax, cr4
    or  rax, 0xa0
    mov cr4, rax

    // Set the page table. The application processors use
    // the same page table as the bootstrap processor's
    // boot phase page table.
    mov rax, __boot_page_table_pointer
    mov cr3, rax

    // Update RIP to use the virtual address.
    push 0x8
    mov rax, offset ap_long_mode
    push rax
    retfq

// This is a pointer to be filled by the BSP when boot stacks
// of all APs are allocated and initialized.
.global __ap_boot_stack_array_pointer
.align 8
__ap_boot_stack_array_pointer:
    .skip 8

ap_long_mode:

    // The local APIC ID is in the RDI.
    mov rax, rdi
    shl rax, 3

    // Setup the stack.
    mov rbx, [__ap_boot_stack_array_pointer]
    mov rsp, [rbx + rax]
    xor rbp, rbp

    // Go to Rust code.
    mov rax, offset ap_early_entry
    call rax
