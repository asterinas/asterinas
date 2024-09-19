/* SPDX-License-Identifier: MPL-2.0 */

.text
.global context_switch
.code64
context_switch: # (cur: *mut TaskContext, nxt: *TaskContext)
  # Save cur's register
  mov rax, [rsp] # return address
  mov [rdi + 56], rax # 56 = offsetof(Context, rip)
  mov [rdi + 0], rsp
  mov [rdi + 8], rbx
  mov [rdi + 16], rbp
  mov [rdi + 24], r12
  mov [rdi + 32], r13
  mov [rdi + 40], r14
  mov [rdi + 48], r15
  rdfsbase r15
  mov [rdi + 64], r15
  # Restore nxt's registers
  mov r15, [rsi + 64]
  wrfsbase r15
  mov rsp, [rsi + 0]
  mov rbx, [rsi + 8]
  mov rbp, [rsi + 16]
  mov r12, [rsi + 24]
  mov r13, [rsi + 32]
  mov r14, [rsi + 40]
  mov r15, [rsi + 48]
  mov rax, [rsi + 56] # restore return address
  mov [rsp], rax # for stack balance, must use mov instead of push
  ret
