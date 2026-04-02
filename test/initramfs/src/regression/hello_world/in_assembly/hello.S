# SPDX-License-Identifier: MPL-2.0

.global _start

.section .text
_start:
    call    print_message
    call    print_message
    call    print_message
    mov     $60, %rax               # syscall number of exit
    mov     $0, %rdi                # exit code
    syscall     
get_pid: 
    mov     $39, %rax
    syscall
    ret
print_message:
    mov     $1, %rax                # syscall number of write
    mov     $1, %rdi                # stdout
    mov     $message, %rsi          # address of message         
    mov     $message_end, %rdx
    sub     %rsi, %rdx              # calculate message len
    syscall
    ret
.section .rodata            
message:
    .ascii  "Hello, world\n"
message_end:
