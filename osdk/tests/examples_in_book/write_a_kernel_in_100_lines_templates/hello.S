# SPDX-License-Identifier: MPL-2.0

.global _start                      # entry point
.section .text                      # code section
_start:
    mov     $1, %rax                # syscall number of write
    mov     $1, %rdi                # stdout
    mov     $message, %rsi          # address of message         
    mov     $message_end, %rdx
    sub     %rsi, %rdx              # calculate message len
    syscall
    mov     $60, %rax               # syscall number of exit, move it to rax
    mov     $0, %rdi                # exit code, move it to rdi
    syscall  

.section .rodata                    # read only data section
message:
    .ascii  "Hello, world\n"
message_end: