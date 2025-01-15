# SPDX-License-Identifier: MPL-2.0

.global _start

.section .text
_start:
    call    print_asterinas
    mov     $60, %rax               # syscall number of exit
    mov     $0, %rdi                # exit code
    syscall                         # 执行 exit 系统调用

print_asterinas:
    mov     $5070, %rax             # syscall number of write
    mov     $0x000000ff, %rdi       # 设置第一个参数为 0x00000000
    syscall                         # 执行系统调用
    ret

.section .rodata
