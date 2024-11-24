/* SPDX-License-Identifier: MPL-2.0 OR MIT
 *
 * The original source code is from [trapframe-rs](https://github.com/rcore-os/trapframe-rs),
 * which is released under the following license:
 *
 * SPDX-License-Identifier: MIT
 *
 * Copyright (c) 2020 - 2024 Runji Wang
 *
 * We make the following new changes:
 * * Add the `trap_handler_table`.
 *
 * These changes are released under the following license:
 *
 * SPDX-License-Identifier: MPL-2.0
 */

.code64

.equ NUM_INT, 256

.altmacro
.macro DEF_HANDLER, i
.Ltrap_handler_\i:
.if \i == 8 || (\i >= 10 && \i <= 14) || \i == 17
    # error code pushed by CPU
    push    \i          # interrupt vector
    jmp     trap_common
.else
    push    0           # fill in error code in TrapFrame
    push    \i          # interrupt vector
    jmp     trap_common
.endif
.endm

.section .text
_trap_handlers:
.set i, 0
.rept NUM_INT
    DEF_HANDLER %i
    .set i, i + 1
.endr

.macro DEF_TABLE_ENTRY, i
    .quad .Ltrap_handler_\i
.endm

.section .rodata
.global trap_handler_table
trap_handler_table:
.set i, 0
.rept NUM_INT
    DEF_TABLE_ENTRY %i
    .set i, i + 1
.endr

.section .text
.global trap_common
trap_common:
    cld                     # clear DF before calling/returning to any C function to conform to x86-64 calling convention
    push rax
    mov ax, [rsp + 4*8]     # load cs
    and ax, 0x3             # test
    jz __from_kernel        # continue trap

__from_user:
    /*
    kernel stack:
    - ptr to GeneralRegs
    - ss
    - rsp
    - rflags
    - cs
    - rip
    - error code
    - trap num
    - rax
    */
    swapgs                  # swap in kernel gs
    mov rax, [rsp + 6*8]    # rax = user rsp
    mov gs:12, rax          # store user rsp -> scratch at TSS.sp1

    mov rsp, [rsp + 8*8]    # load rsp -> GeneralRegs
    add rsp, 22*8           # rsp -> top of GeneralRegs
    mov rax, gs:4           # rax = kernel stack

    # push trap_num, error_code
    push [rax - 6*8]        # push error_code
    push [rax - 7*8]        # push trap_num
    sub rsp, 16             # skip fsbase, gsbase
    # push general registers
    push [rax - 3*8]        # push rflags
    push [rax - 5*8]        # push rip
    mov rax, [rax - 8*8]    # pop rax
    jmp trap_syscall_entry

__from_kernel:
    /*
    kernel stack:
    - rflags
    - cs
    - rip
    - error code
    - trap num
    - rax
    */
    pop rax
    push 0
    push r15
    push r14
    push r13
    push r12
    push r11
    push r10
    push r9
    push r8
    lea r8, [rsp + 13*8]
    push r8                 # push rsp
    push rbp
    push rdi
    push rsi
    push rdx
    push rcx
    push rbx
    push rax

    mov rdi, rsp
    call trap_handler

.global trap_return
trap_return:
    pop rax
    pop rbx
    pop rcx
    pop rdx
    pop rsi
    pop rdi
    pop rbp
    pop r8                  # skip rsp
    pop r8
    pop r9
    pop r10
    pop r11
    pop r12
    pop r13
    pop r14
    pop r15

    # skip padding, trap_num, error_code
    add rsp, 24

    iretq
