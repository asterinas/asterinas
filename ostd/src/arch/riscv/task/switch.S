/* SPDX-License-Identifier: MPL-2.0 */

.text
.global context_switch
.global first_context_switch
.global kernel_task_entry_wrapper

context_switch: # (nxt: *const TaskContext, cur: *mut TaskContext)
  # Save cur's register
  sd  ra, 0x68(a1) # return address
  sd  sp, 0x0(a1)
  sd  s0, 0x8(a1)
  sd  s1, 0x10(a1)
  sd  s2, 0x18(a1)
  sd  s3, 0x20(a1)
  sd  s4, 0x28(a1)
  sd  s5, 0x30(a1)
  sd  s6, 0x38(a1)
  sd  s7, 0x40(a1)
  sd  s8, 0x48(a1)
  sd  s9, 0x50(a1)
  sd  s10, 0x58(a1)
  sd  s11, 0x60(a1)
  # Fallthrough

first_context_switch: # (nxt: *const TaskContext)
  # Restore nxt's registers
  ld  ra, 0x68(a0) # return address
  ld  sp, 0x0(a0)
  ld  s0, 0x8(a0)
  ld  s1, 0x10(a0)
  ld  s2, 0x18(a0)
  ld  s3, 0x20(a0)
  ld  s4, 0x28(a0)
  ld  s5, 0x30(a0)
  ld  s6, 0x38(a0)
  ld  s7, 0x40(a0)
  ld  s8, 0x48(a0)
  ld  s9, 0x50(a0)
  ld  s10, 0x58(a0)
  ld  s11, 0x60(a0)
  ret

kernel_task_entry_wrapper:
  .cfi_startproc
  .cfi_undefined ra # mark return address as undefined to indicate end of call stack
  call kernel_task_entry
  .cfi_endproc
