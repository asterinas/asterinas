# SPDX-License-Identifier: MPL-2.0

.intel_syntax noprefix
.code16
.global _start
.section .text
_start:
    xor ax, ax
    mov ds, ax
    cld
    mov si, offset message
    mov dx, 0x3f8

.Lprint:
    lodsb
    test al, al
    jz .Lhalt
    out dx, al
    jmp .Lprint

.Lhalt:
    hlt

message:
    .asciz "Hello World\n"
