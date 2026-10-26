# SPDX-License-Identifier: MPL-2.0

    .section .text
    .globl _start
_start:
    la      t0, message
    li      t1, 0x1000         # t1 = serial MMIO address 0x1000

.Lprint:
    lbu     t2, 0(t0)          
    beqz    t2, .Lhalt         
    sb      t2, 0(t1)          
    addi    t0, t0, 1         
    j       .Lprint

.Lhalt:
    wfi
    j       .Lhalt

    .section .rodata
message:
    .asciz "Hello World\n"

