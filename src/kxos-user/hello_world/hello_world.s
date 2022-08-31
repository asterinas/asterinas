.global _start

.section .text
_start:
    call print_message
    call print_message
    call print_message
    mov     $60, %rax               # syscall number of exit
    mov     $0, %rdi                 # exit code
    syscall     
print_message:
    mov     $1, %rax                # syscall number of write
    mov     $1, %rdi                # stdout
    mov     $message, %rsi          # address of message
    mov     $message, %r11          
    mov     $message_end, %r12
    sub     %r11, %r12               # calculate message len
    mov     %r12, %rdx               # number of bytes
    syscall
    ret
.section .rodata            
message:
    .ascii  "Hello, world\n"
message_end:
