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

# Constants / Macros defined in Rust code:
#   XLENB
#   LOAD
#   STORE

    .section .text
    .global trap_entry
    .balign 4
trap_entry:
    # If coming from userspace, preserve the user stack pointer and load
    # the kernel stack pointer. If we came from the kernel, sscratch
    # will contain 0, and we should continue on the current stack.
    csrrw sp, sscratch, sp
    bnez sp, trap_from_user
trap_from_kernel:
    csrr sp, sscratch
    addi sp, sp, -34 * XLENB
    # sscratch = previous-sp, sp = kernel-sp
trap_from_user:
    # save general registers except sp(x2)
    STORE_SP x1, 1
    STORE_SP x3, 3
    STORE_SP x4, 4
    STORE_SP x5, 5
    STORE_SP x6, 6
    STORE_SP x7, 7
    STORE_SP x8, 8
    STORE_SP x9, 9
    STORE_SP x10, 10
    STORE_SP x11, 11
    STORE_SP x12, 12
    STORE_SP x13, 13
    STORE_SP x14, 14
    STORE_SP x15, 15
    STORE_SP x16, 16
    STORE_SP x17, 17
    STORE_SP x18, 18
    STORE_SP x19, 19
    STORE_SP x20, 20
    STORE_SP x21, 21
    STORE_SP x22, 22
    STORE_SP x23, 23
    STORE_SP x24, 24
    STORE_SP x25, 25
    STORE_SP x26, 26
    STORE_SP x27, 27
    STORE_SP x28, 28
    STORE_SP x29, 29
    STORE_SP x30, 30
    STORE_SP x31, 31

    # save sp, sstatus, sepc
    csrrw t0, sscratch, x0  # sscratch = 0 (kernel)
    csrr t1, sstatus
    csrr t2, sepc
    STORE_SP t0, 2          # save sp
    STORE_SP t1, 32         # save sstatus
    STORE_SP t2, 33         # save sepc

    li t0, 3 << 13
    or t1, t1, t0           # sstatus.FS = Dirty (3)
    csrw sstatus, t1

    andi t1, t1, 1 << 8     # sstatus.SPP == 1
    beqz t1, end_trap_from_user
end_trap_from_kernel:
    mv a0, sp               # first arg is TrapFrame
    la ra, trap_return      # set return address
    j trap_handler

end_trap_from_user:
    # load callee-saved registers
    LOAD_SP sp, 0
    LOAD_SP s0, 0
    LOAD_SP s1, 1
    LOAD_SP s2, 2
    LOAD_SP s3, 3
    LOAD_SP s4, 4
    LOAD_SP s5, 5
    LOAD_SP s6, 6
    LOAD_SP s7, 7
    LOAD_SP s8, 8
    LOAD_SP s9, 9
    LOAD_SP s10, 10
    LOAD_SP s11, 11
    LOAD_SP ra, 12
    # not callee-saved, but is used to store mhartid
    LOAD_SP gp, 13
    addi sp, sp, 14 * XLENB

    ret

.global run_user
run_user:
    # save callee-saved registers
    addi sp, sp, -14 * XLENB
    STORE_SP s0, 0
    STORE_SP s1, 1
    STORE_SP s2, 2
    STORE_SP s3, 3
    STORE_SP s4, 4
    STORE_SP s5, 5
    STORE_SP s6, 6
    STORE_SP s7, 7
    STORE_SP s8, 8
    STORE_SP s9, 9
    STORE_SP s10, 10
    STORE_SP s11, 11
    STORE_SP ra, 12
    # not callee-saved, but is used to store mhartid
    STORE_SP gp, 13

    mv t0, sp
    mv sp, a0
    STORE_SP t0, 0          # save kernel-sp
    csrw sscratch, sp       # sscratch = bottom of trap frame

trap_return:
    LOAD_SP t0, 32          # t0 = sstatus
    LOAD_SP t1, 33          # t1 = sepc
    csrw sstatus, t0        # load sstatus
    csrw sepc, t1           # load sepc

    # restore general registers except sp(x2)
    LOAD_SP x1, 1
    LOAD_SP x3, 3
    LOAD_SP x4, 4
    LOAD_SP x5, 5
    LOAD_SP x6, 6
    LOAD_SP x7, 7
    LOAD_SP x8, 8
    LOAD_SP x9, 9
    LOAD_SP x10, 10
    LOAD_SP x11, 11
    LOAD_SP x12, 12
    LOAD_SP x13, 13
    LOAD_SP x14, 14
    LOAD_SP x15, 15
    LOAD_SP x16, 16
    LOAD_SP x17, 17
    LOAD_SP x18, 18
    LOAD_SP x19, 19
    LOAD_SP x20, 20
    LOAD_SP x21, 21
    LOAD_SP x22, 22
    LOAD_SP x23, 23
    LOAD_SP x24, 24
    LOAD_SP x25, 25
    LOAD_SP x26, 26
    LOAD_SP x27, 27
    LOAD_SP x28, 28
    LOAD_SP x29, 29
    LOAD_SP x30, 30
    LOAD_SP x31, 31
    # restore sp last
    LOAD_SP x2, 2

    # return from supervisor call
    sret
