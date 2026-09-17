# SPDX-License-Identifier: MPL-2.0

.code16
.global _start
.section .text
_start:
    xorw %ax, %ax
    movw %ax, %ds
    cld
    movw $message, %si
    movw $0x3f8, %dx

.Lprint:
    lodsb
    testb %al, %al
    jz .Lhalt
    outb %al, %dx
    jmp .Lprint

.Lhalt:
    hlt

message:
    .asciz "Hello World\n"
