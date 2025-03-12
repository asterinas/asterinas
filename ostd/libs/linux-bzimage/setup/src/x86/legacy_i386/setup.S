/* SPDX-License-Identifier: MPL-2.0 */

// The load address of the setup section is CODE32_START (0x100000).
// See the linker script.
.section ".setup", "ax"

.code32
.global entry_legacy32
entry_legacy32:
    // This is the 32-bit Linux legacy entry point.
    //
    // Arguments:
    //  RSI: struct boot_params *bp

    // We don't do reloactions, so let's make sure we're at the right address.
    // Otherwise, we'd better stop here.
    lea esp, [esi + 0x1e8]  // scratch: u32 (offset 0x1e4)
    call load_address
load_address:
    pop eax
    cmp eax, offset load_address
    jne halt

    // Set up the stack.
    mov esp, offset __stack_top

    // Load the GDT.
    push 8        // 32-bit code
    mov eax, offset gdt_loaded
    push eax
    lgdt [gdtr]
    retf
gdt_loaded:
    mov ax, 16    // 32-bit data
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    // Call the Rust main routine.
    push esi
    call main_legacy32

    // The main routine should not return. If it does, there is nothing we can
    // do but stop the machine.
    jmp halt

// All other types of entry points are not enabled in the header.
// So we don't care about them.

halt:
    hlt
    jmp halt

// GDT. We define GDT ourselves to ensure that the GDT page will not be
// accidentally overwritten by the allocated memory.
.rodata
.align 16
gdtr:
    .word gdt_end - gdt - 1
    .long gdt
.align 16
gdt:
    .quad 0x0000000000000000 // 0:  null descriptor
    .quad 0x00cf9a000000ffff // 8:  32-bit code segment
    .quad 0x00cf92000000ffff // 16: 32-bit data segment
gdt_end:

// A small stack for the setup code.
.bss
.align 8
__stack_bottom:
    .space 0x1000
__stack_top:
