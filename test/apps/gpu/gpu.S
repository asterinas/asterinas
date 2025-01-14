# SPDX-License-Identifier: MPL-2.0

.global _start

.section .text
_start:
    call    print_asterinas
    mov     $60, %rax               # syscall number of exit
    mov     $0, %rdi                # exit code
    syscall     

print_asterinas:
    mov     $5090, %rax                # syscall number of write
    syscall
    ret


.section .rodata 

