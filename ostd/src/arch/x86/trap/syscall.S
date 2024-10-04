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
 * * Skip saving/restoring the fsgsbase registers.
 *
 * These changes are released under the following license:
 *
 * SPDX-License-Identifier: MPL-2.0
 */

.code64

.text
    # extern "sysv64" fn syscall_return(&mut GeneralRegs)
.global syscall_return
syscall_return:
    # disable interrupt
    cli

    # save callee-saved registers
    push r15
    push r14
    push r13
    push r12
    push rbp
    push rbx

    push rdi                # keep rsp 16 bytes align
    mov gs:4, rsp           # store kernel rsp -> TSS.sp0
    mov rsp, rdi            # set rsp -> GeneralRegs

    # restore user gsbase
    swapgs

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
    # rip
    # rflags
    # fsbase
    # gsbase
    # trap_num
    # error_code

    # determain sysret or iret
    cmp dword ptr [rsp + 4*8], 0x100  # syscall?
    je sysret
iret:
    # construct trap frame
    push [USER_SS]          # push ss
    push [rsp - 8*8]        # push rsp
    push [rsp + 3*8]        # push rflags
    push [USER_CS]          # push cs
    push [rsp + 4*8]        # push rip

    iretq

sysret:
    pop rcx                 # rcx = rip
    pop r11                 # r11 = rflags
    mov rsp, [rsp - 11*8]   # load rsp

    sysretq

    # sysretq instruction do:
    # - load cs, ss
    # - load rflags <- r11
    # - load rip <- rcx

.global syscall_entry
syscall_entry:
    # syscall instruction do:
    # - load cs
    # - store rflags -> r11
    # - mask rflags
    # - store rip -> rcx
    # - load rip

    swapgs                  # swap in kernel gs
    mov gs:12, rsp          # store user rsp -> scratch at TSS.sp1
    mov rsp, gs:4           # load kernel rsp <- TSS.sp0
    pop rsp                 # load rsp -> GeneralRegs
    add rsp, 21*8           # rsp -> error code of GeneralRegs

    push 0x100              # push trap_num
    sub rsp, 16             # skip fsbase, gsbase
    # push general registers
    push r11                # push rflags
    push rcx                # push rip

.global trap_syscall_entry
trap_syscall_entry:
    push r15
    push r14
    push r13
    push r12
    push r11
    push r10
    push r9
    push r8
    push gs:12              # push rsp
    push rbp
    push rdi
    push rsi
    push rdx
    push rcx
    push rbx
    push rax

    # restore callee-saved registers
    mov rsp, gs:4           # load kernel rsp <- TSS.sp0
    pop rbx

    pop rbx
    pop rbp
    pop r12
    pop r13
    pop r14
    pop r15

    # go back to Rust
    ret
