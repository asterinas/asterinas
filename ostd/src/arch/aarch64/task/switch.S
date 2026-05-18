// SPDX-License-Identifier: MPL-2.0

// Task context switch for ARM64.
// TaskContext layout (see task/mod.rs):
//   offset 0x00: sp
//   offset 0x08: x19
//   offset 0x10: x20
//   offset 0x18: x21
//   offset 0x20: x22
//   offset 0x28: x23
//   offset 0x30: x24
//   offset 0x38: x25
//   offset 0x40: x26
//   offset 0x48: x27
//   offset 0x50: x28
//   offset 0x58: x29  (frame pointer)
//   offset 0x60: lr   (instruction pointer, set via set_instruction_pointer)

.text

.global context_switch
context_switch:  // (nxt: *const TaskContext in x0, cur: *mut TaskContext in x1)
    // Save current context to cur.
    mov   x2, sp
    str   x2, [x1, #0x00]
    stp   x19, x20, [x1, #0x08]
    stp   x21, x22, [x1, #0x18]
    stp   x23, x24, [x1, #0x28]
    stp   x25, x26, [x1, #0x38]
    stp   x27, x28, [x1, #0x48]
    stp   x29, x30, [x1, #0x58]
    // Fall through to first_context_switch.

.global first_context_switch
first_context_switch:  // (nxt: *const TaskContext in x0)
    // Restore next context from nxt.
    ldp   x19, x20, [x0, #0x08]
    ldp   x21, x22, [x0, #0x18]
    ldp   x23, x24, [x0, #0x28]
    ldp   x25, x26, [x0, #0x38]
    ldp   x27, x28, [x0, #0x48]
    ldp   x29, x30, [x0, #0x58]
    ldr   x2, [x0, #0x00]
    mov   sp, x2
    ret

.global kernel_task_entry_wrapper
kernel_task_entry_wrapper:
    bl    kernel_task_entry
