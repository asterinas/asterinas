/* SPDX-License-Identifier: MPL-2.0 */

.text
.code64
.global context_switch
.global first_context_switch

context_switch: # (nxt: *const TaskContext, cur: *mut TaskContext)
  # Save cur's register
  mov rax, [rsp] # return address
  mov [rsi + 56], rax # 56 = offsetof(TaskContext, rip)
  mov [rsi + 0], rsp
  mov [rsi + 8], rbx
  mov [rsi + 16], rbp
  mov [rsi + 24], r12
  mov [rsi + 32], r13
  mov [rsi + 40], r14
  mov [rsi + 48], r15
  rdfsbase r15
  mov [rsi + 64], r15 # 64 = offsetof(TaskContext, fsbase)
  # Fallthrough

first_context_switch: # (nxt: *const TaskContext)
  # Restore nxt's registers
  mov r15, [rdi + 64]
  wrfsbase r15
  mov rsp, [rdi + 0]
  mov rbx, [rdi + 8]
  mov rbp, [rdi + 16]
  mov r12, [rdi + 24]
  mov r13, [rdi + 32]
  mov r14, [rdi + 40]
  mov r15, [rdi + 48]
  mov rax, [rdi + 56]
  # for stack balance, we have to use mov instead of push here
  mov qword ptr [rsp + 8], 0 # kernel_task_entry's return address
  mov [rsp], rax             # first_context_switch's return address
  ret
