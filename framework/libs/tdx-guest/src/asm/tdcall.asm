# SPDX-License-Identifier: BSD-3-Clause
# Copyright(c) 2023-2024 Intel Corporation.

.section .text

# Arguments offsets in TdVmcallArgs struct
.equ TDCALL_ARG_RAX, 0x0
.equ TDCALL_ARG_RCX, 0x8
.equ TDCALL_ARG_RDX, 0x10
.equ TDCALL_ARG_R8,  0x18
.equ TDCALL_ARG_R9,  0x20
.equ TDCALL_ARG_R10, 0x28
.equ TDCALL_ARG_R11, 0x30
.equ TDCALL_ARG_R12, 0x38
.equ TDCALL_ARG_R13, 0x40

# asm_td_call -> u64 (
#   args: *mut TdcallArgs,  //rcx
# )
.global asm_td_call
asm_td_call:
        endbr64
        # Save the registers accroding to MS x64 calling convention
        push rbp
        mov rbp, rsp
        push r15
        push r14
        push r13
        push r12
        push rbx
        push rsi
        push rdi

        # Use RDI to save RCX value
        mov rdi, rcx

        # Test if input pointer is valid
        test rdi, rdi
        jz td_call_exit

        # Copy the input operands from memory to registers 
        mov rax, [rdi + TDCALL_ARG_RAX]
        mov rcx, [rdi + TDCALL_ARG_RCX]
        mov rdx, [rdi + TDCALL_ARG_RDX]
        mov r8,  [rdi + TDCALL_ARG_R8]
        mov r9,  [rdi + TDCALL_ARG_R9]
        mov r10, [rdi + TDCALL_ARG_R10]
        mov r11, [rdi + TDCALL_ARG_R11]
        mov r12, [rdi + TDCALL_ARG_R12]
        mov r13, [rdi + TDCALL_ARG_R13]

        # tdcall
        .byte 0x66,0x0f,0x01,0xcc

        # Exit if tdcall reports failure.
        test rax, rax
        jnz td_call_exit

        # Copy the output operands from registers to the struct
        mov [rdi + TDCALL_ARG_RAX], rax
        mov [rdi + TDCALL_ARG_RCX], rcx
        mov [rdi + TDCALL_ARG_RDX], rdx
        mov [rdi + TDCALL_ARG_R8],  r8
        mov [rdi + TDCALL_ARG_R9],  r9
        mov [rdi + TDCALL_ARG_R10], r10
        mov [rdi + TDCALL_ARG_R11], r11
        mov [rdi + TDCALL_ARG_R12], r12
        mov [rdi + TDCALL_ARG_R13], r13

td_call_exit:
        # Pop out saved registers from stack
        pop rdi
        pop rsi
        pop rbx
        pop r12
        pop r13
        pop r14
        pop r15
        pop rbp

        ret
