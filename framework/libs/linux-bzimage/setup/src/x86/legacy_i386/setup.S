/* SPDX-License-Identifier: MPL-2.0 */

// 32-bit setup code starts here, and will be loaded at CODE32_START.
.section ".setup", "ax"
.code32
.global start_of_setup32
start_of_setup32:
    mov eax, offset __stack_top
    mov esp, eax
    mov eax, offset halt
    push eax                        # the return address
    mov ebp, esp
    add ebp, -4
    push ebp
    mov ebp, esp

// The rust entrypoint of the bzImage
.extern _bzimage_entry_32
    push esi                        # the boot_params pointer
    call _bzimage_entry_32

    // Unreachable here.
halt:
    hlt
    jmp halt

// A small stack for the 32-bit code.
.section ".stack", "aw"
.align 8
    .space 0x1000
__stack_top:
