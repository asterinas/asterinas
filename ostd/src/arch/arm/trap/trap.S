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
 * * Adjust some symbol names.
 * * Revise some comments.
 *
 * These changes are released under the following license:
 *
 * SPDX-License-Identifier: MPL-2.0
 */

.text

trap_common:
    # x30 and x0 are saved in __vectors
    # x0 is trap num now
    # skip __reversed
    str     x29, [sp, #-16]!
    stp     x27, x28, [sp, #-16]!
    stp     x25, x26, [sp, #-16]!
    stp     x23, x24, [sp, #-16]!
    stp     x21, x22, [sp, #-16]!
    stp     x19, x20, [sp, #-16]!
    stp     x17, x18, [sp, #-16]!
    stp     x15, x16, [sp, #-16]!
    stp     x13, x14, [sp, #-16]!
    stp     x11, x12, [sp, #-16]!
    stp     x9, x10, [sp, #-16]!
    stp     x7, x8, [sp, #-16]!
    stp     x5, x6, [sp, #-16]!
    stp     x3, x4, [sp, #-16]!
    stp     x1, x2, [sp, #-16]!

    # skip tpidr and sp
    add     sp, sp, #-16

    # read spsr and elr
    mrs     x2, spsr_el1
    mrs     x1, elr_el1
    stp     x1, x2, [sp, #-16]!

    # save trap num
    str     x0, [sp, #-16]!

    # check source is 2
    mov     x1, #0x3
    and     x1, x1, x0
    cmp     x1, #2
    beq     _trap_from_user

_trap_from_kernel:
    # read tpidr and sp
    mrs     x2, tpidr_el1
    add     x1, sp, #38*8
    stp     x1, x2, [sp, #32]

    # go to rust
    mov     x0, sp
    bl      trap_handler

    # load tpidr
    ldr     x1, [sp, #40]
    msr     tpidr_el1, x1

    b       _trap_return

_trap_from_user:
    # read tpidr and sp
    mrs     x2, tpidr_el0
    mrs     x1, sp_el0
    stp     x1, x2, [sp, #32]

    # read sp
    ldr     x2, [sp, #8]
    mov     sp, x2

    # load callee-saved registers
    ldp     x19, x20, [sp], #16
    ldp     x21, x22, [sp], #16
    ldp     x23, x24, [sp], #16
    ldp     x25, x26, [sp], #16
    ldp     x27, x28, [sp], #16
    ldp     x29, x30, [sp], #16

    ret

.macro HANDLER source kind
    .balign 128
    # sp is set to sp_el1 upon trap
    stp     lr, x0, [sp, #-16]!
    mov     x0, #\source
    movk    x0, #\kind, lsl #16
    b       trap_common
.endm

.balign 2048
.global __vectors
__vectors:
    HANDLER 0 0
    HANDLER 0 1
    HANDLER 0 2
    HANDLER 0 3
    HANDLER 1 0
    HANDLER 1 1
    HANDLER 1 2
    HANDLER 1 3
    HANDLER 2 0
    HANDLER 2 1
    HANDLER 2 2
    HANDLER 2 3
    HANDLER 3 0
    HANDLER 3 1
    HANDLER 3 2
    HANDLER 3 3

.global run_user
run_user: # (regs: &mut RawUserContext)
    # x0 points to TrapFrame
    # save callee-saved registers x19-x29
    stp     x29, x30, [sp, #-16]!
    stp     x27, x28, [sp, #-16]!
    stp     x25, x26, [sp, #-16]!
    stp     x23, x24, [sp, #-16]!
    stp     x21, x22, [sp, #-16]!
    stp     x19, x20, [sp, #-16]!

    # save kernel sp to TrapFrame
    mov     x1, sp
    mov     sp, x0
    str     x1, [sp, #8]

    # load sp and tpidr
    ldp     x1, x2, [sp, #32]
    msr     sp_el0, x1
    msr     tpidr_el0, x2

_trap_return:
    # sp points to TrapFrame
    # skip trap num, don't restore
    add     sp, sp, #16

    # elr and spsr
    ldp     x1, x2, [sp], #16
    msr     elr_el1, x1
    msr     spsr_el1, x2

    # skip sp and tpidr
    add     sp, sp, #16

    # general purpose registers
    ldp     x1, x2, [sp], #16
    ldp     x3, x4, [sp], #16
    ldp     x5, x6, [sp], #16
    ldp     x7, x8, [sp], #16
    ldp     x9, x10, [sp], #16
    ldp     x11, x12, [sp], #16
    ldp     x13, x14, [sp], #16
    ldp     x15, x16, [sp], #16
    ldp     x17, x18, [sp], #16
    ldp     x19, x20, [sp], #16
    ldp     x21, x22, [sp], #16
    ldp     x23, x24, [sp], #16
    ldp     x25, x26, [sp], #16
    ldp     x27, x28, [sp], #16
    ldr     x29, [sp], #16
    ldp     lr, x0, [sp], #16

    eret
