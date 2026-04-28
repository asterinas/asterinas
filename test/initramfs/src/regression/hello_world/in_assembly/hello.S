# SPDX-License-Identifier: MPL-2.0

.global _start

.section .text
_start:
#if defined(__x86_64__)
    call    print_message
    call    print_message
    call    print_message
    mov     $60, %rax               # syscall number of exit
    mov     $0, %rdi                # exit code
    syscall
print_message:
    mov     $1, %rax                # syscall number of write
    mov     $1, %rdi                # stdout
    mov     $message, %rsi          # address of message
    mov     $message_end, %rdx
    sub     %rsi, %rdx              # calculate message len
    syscall
    ret
#elif defined(__riscv) && __riscv_xlen == 64
    call    print_message
    call    print_message
    call    print_message
    li      a7, 93                  # syscall number of exit
    li      a0, 0                   # exit code
    ecall
print_message:
    li      a7, 64                  # syscall number of write
    li      a0, 1                   # stdout
    lla     a1, message             # address of message
    lla     a2, message_end
    sub     a2, a2, a1              # calculate message len
    ecall
    ret
#else
#error "unsupported architecture"
#endif

.section .rodata
message:
    .ascii  "Hello, world\n"
message_end:
