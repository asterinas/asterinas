/* SPDX-License-Identifier: MPL-2.0 */

.text

.global context_switch
.type context_switch, @function
context_switch: # (nxt: *const TaskContext, cur: *mut TaskContext)
  stp x19, x20, [x1], #16
  stp x21, x22, [x1], #16
  stp x23, x24, [x1], #16
  stp x25, x26, [x1], #16
  stp x27, x28, [x1], #16
  mov x2, sp
  stp x29, x2, [x1], #16
  str x30, [x1]
  # fallthrough

.global first_context_switch
.type first_context_switch, @function
first_context_switch: # (nxt: *const TaskContext)
  ldp x19, x20, [x0], #16
  ldp x21, x22, [x0], #16
  ldp x23, x24, [x0], #16
  ldp x25, x26, [x0], #16
  ldp x27, x28, [x0], #16
  ldp x29, x2, [x0], #16
  mov sp, x2
  ldr x30, [x0]

  ret
.size context_switch, .-context_switch
.size first_context_switch, .-first_context_switch

.global kernel_task_entry_wrapper
.type kernel_task_entry_wrapper, @function
kernel_task_entry_wrapper:
  .cfi_startproc
  .cfi_undefined 30 // mark return address as undefined to indicate end of call stack
  bl kernel_task_entry
  .cfi_endproc
.size kernel_task_entry_wrapper, .-kernel_task_entry_wrapper
