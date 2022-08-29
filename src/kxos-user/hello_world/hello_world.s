global _start

section .text

_start:
  mov rax, 1        ; syswrite
  mov rdi, 1        ; fd
  mov rsi, msg      ; "Hello, world!\n",
  mov rdx, msglen   ; sizeof("Hello, world!\n")
  syscall           

  mov rax, 60       ; sys_exit
  mov rdi, 0        ; exit_code
  syscall           

section .rodata
  msg: db "Hello, world!", 10
  msglen: equ $ - msg