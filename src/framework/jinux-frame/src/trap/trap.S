.data
.align 4
TSS:
  .space 104

.text

.macro save
  push r11
  push r10
  push r9
  push r8
  push rdi
  push rsi
  push rdx
  push rcx
  push rax
.endm

.macro restore
  pop rax
  pop rcx
  pop rdx
  pop rsi
  pop rdi
  pop r8
  pop r9
  pop r10
  pop r11
.endm

.global __trap_entry
__trap_entry:
  # 保存寄存器
  push r15
  push r14
  push r13
  push r12
  push rbp
  push rbx
  # mov rdi, 0
  push rdi
  save
  # save cr2
  mov rdi, cr2
  push rdi
  # 将栈指针当成参数传入trap_handler
  mov rdi, rsp
  call trap_handler
__trap_return:
  # judge whether the trap from kernel mode
  mov rax, [rsp + 160] # 160 = offsetof(TrapFrame, cs)
  and rax, 0x3
  jz __from_kernel
  lea rax, [rsp + 192] # prepare new TSS.sp0, 192 = sizeof(TrapFrame)
  mov [TSS + rip + 4], rax
__from_kernel:
  add rsp, 8 # skip cr2
  restore
  add rsp,8 # skip rsp in callee
  pop rbx
  pop rbp
  pop r12
  pop r13
  pop r14
  pop r15

  add rsp, 16 # skip TrapFrame.err and id
  iretq




.global syscall_entry
syscall_entry:
  # syscall instruction do:
  # - load cs, ss from STAR MSR
  # - r11 <- rflags, mask rflags from RFMASK MSR
  # - rcx <- rip, load rip from LSTAR MSR

  # temporarily store user rsp into TSS.sp0 and load kernel rsp from it.
  xchg rsp, [TSS + rip + 4]
  push r15
  push r14
  push r13
  push r12
  push rbp
  push rbx
  push [TSS + rip + 4] # store user rsp into SyscallFrame.rsp
  save
  mov rdi, rsp
  call syscall_handler
  mov [rsp], rax # CallerRegs.rax is at offset 0
  jmp __syscall_return

.global syscall_return
syscall_return: # (SyscallFrame *)
  mov rsp, rdi
__syscall_return:
  lea rax, [rsp + 128] # prepare new TSS.sp0, 128 = sizeof(SyscallFrame)
  # store the rsp in TSS
  mov [TSS + rip + 4], rax
  restore
  mov rbx, [rsp + 8]
  mov rbp, [rsp + 16]
  mov r12, [rsp + 24]
  mov r13, [rsp + 32]
  mov r14, [rsp + 40]
  mov r15, [rsp + 48]
  mov rsp, [rsp + 0]
  sysretq

.global syscall_switch_to_user_space
syscall_switch_to_user_space: # (cpu_context: *CpuContext,reg: *SyscallFrame)
  # mov rflag, [rdi+136]
  mov rdi, rsi
  jmp syscall_return

.global trap_switch_to_user_space
trap_switch_to_user_space: # (cpu_context: *CpuContext,reg: *TrapFrame)
  # mov rflag, [rdi+136]
  mov rdi, rsi
  mov rsp, rdi
  jmp __trap_return


