# SPDX-License-Identifier: MPL-2.0

.global _start

.section .text
_start:
    call    print_hello_world
    mov     $57, %rax                   # syscall number of fork
    syscall

    cmp     $0, %rax
    je      _child                      # child process
    jmp     _parent                     # parent process
_parent:
    call    wait_child
    call    get_pid 
    call    print_parent_message
    call    exit
_child: 
    call    get_pid
    call    print_child_message
    call    exit
wait_child:
    mov     %rax, %rdi                  # child process id
_loop:
    mov     $61, %rax                   # syscall number of wait4
    mov     $0, %rsi                    # exit status address
    mov     $1, %rdx                    # WNOHANG
    syscall
    cmp     %rdi, %rax                  # The return value is the pid of child 
    jne _loop
    ret
exit:
    mov     $60, %rax                   # syscall number of exit
    mov     $0, %rdi                    # exit code
    syscall
get_pid:
    mov     $39, %rax
    syscall
    ret     
print_hello_world:
    mov     $message, %rsi              # address of message
    mov     $message_end, %rdx
    sub     %rsi, %rdx                  # calculate message len
    jmp     _print_message
print_parent_message:
    mov     $message_parent, %rsi       # address of message
    mov     $message_parent_end, %rdx
    sub     %rsi, %rdx                  # calculate message len
    jmp     _print_message
print_child_message:
    mov     $message_child, %rsi        # address of message
    mov     $message_child_end, %rdx
    sub     %rsi, %rdx                  # calculate message len
    jmp     _print_message
# never directly call _print_message
_print_message:
    mov     $1, %rax                    # syscall number of write
    mov     $1, %rdi                    # stdout
    syscall
    ret
.section .rodata        
message:
    .ascii  "Hello, world in fork\n"
message_end:
message_parent:
    .ascii "Hello world from parent\n"
message_parent_end:
message_child:
    .ascii "Hello world from child\n"
message_child_end:
