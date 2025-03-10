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

// A small stack for the setup code.
.bss
.align 8
__stack_bottom:
    .space 0x1000
__stack_top:
