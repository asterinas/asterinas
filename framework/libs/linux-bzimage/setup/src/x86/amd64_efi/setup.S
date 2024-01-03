/* SPDX-License-Identifier: MPL-2.0 */

.section ".setup", "ax"
.code64
// start_of_setup32 should be loaded at CODE32_START, which is our base.
.global start_of_setup32
start_of_setup32:

// `efi_handover_setup_entry64` should be at efi_handover_setup_entry32 + 0x200, but
// we could provide the 32 bit dummy entry point as the 64 bit entry point - 0x200
// since we do not provide 32-bit entry point in the x86_64 specific implementation.
.org 0x210
.global efi_handover_setup_entry
efi_handover_setup_entry:
    // The 3 parameters of is stored in rdi, rsi and rdx (sysv64).
    // Do not use them.

    // Setup the stack.
    lea rsp, [rip + setup_stack_top]
    lea rax, [rip + halt]
    push rax                        # the return address
    mov rbp, rsp
    add rbp, -4
    push rbp
    mov rbp, rsp

.extern efi_handover_entry
    lea rax, [rip + efi_handover_entry]
    call rax

    // Unreachable here.
halt:
    hlt
    jmp halt

// A small stack for the setup code.
.section .data
.align 0x1000 / 8
.global setup_stack
setup_stack:
    .skip 0x1000
.global setup_stack_top
setup_stack_top: