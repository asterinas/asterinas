# SPDX-License-Identifier: BSD-3-Clause
# Copyright(c) 2023-2024 Intel Corporation.

.section .text

# Mask used to control which part of the guest TD GPR and XMM
# state is exposed to the VMM. A bit value of 1 indicates the
# corresponding register is passed to VMM. Refer to TDX Module
# ABI specification section TDG.VP.VMCALL for detail.
# Here we expose R10 - R15 to VMM in td_vm_call()
.equ TDVMCALL_EXPOSE_REGS_MASK, 0xfc00

# TDG.VP.VMCALL leaf number
.equ TDVMCALL, 0

# Arguments offsets in TdVmcallArgs struct
.equ VMCALL_ARG_R10, 0x0
.equ VMCALL_ARG_R11, 0x8
.equ VMCALL_ARG_R12, 0x10
.equ VMCALL_ARG_R13, 0x18
.equ VMCALL_ARG_R14, 0x20
.equ VMCALL_ARG_R15, 0x28

# asm_td_vmcall -> u64 (
#   args: *mut TdVmcallArgs,
# )
.global asm_td_vmcall
asm_td_vmcall:
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
        jz vmcall_exit

        # Copy the input operands from memory to registers
        mov r10, [rdi + VMCALL_ARG_R10]
        mov r11, [rdi + VMCALL_ARG_R11]
        mov r12, [rdi + VMCALL_ARG_R12]
        mov r13, [rdi + VMCALL_ARG_R13]
        mov r14, [rdi + VMCALL_ARG_R14]
        mov r15, [rdi + VMCALL_ARG_R15]

        # Set TDCALL leaf number
        mov rax, TDVMCALL

        # Set exposed register mask
        mov ecx, TDVMCALL_EXPOSE_REGS_MASK

        # TDCALL
       .byte 0x66,0x0f,0x01,0xcc

        # RAX should always be zero for TDVMCALL, panic if it is not.
        test rax, rax
        jnz vmcall_panic

        # Copy the output operands from registers to the struct
        mov [rdi + VMCALL_ARG_R10], r10
        mov [rdi + VMCALL_ARG_R11], r11
        mov [rdi + VMCALL_ARG_R12], r12
        mov [rdi + VMCALL_ARG_R13], r13
        mov [rdi + VMCALL_ARG_R14], r14
        mov [rdi + VMCALL_ARG_R15], r15

        mov rax, r10

vmcall_exit:
        # Clean the registers that are exposed to VMM to
        # protect against speculative attack, others will
        # be restored to the values saved in stack
        xor r10, r10
        xor r11, r11

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

vmcall_panic:
        ud2
