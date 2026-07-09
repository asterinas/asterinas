/* SPDX-License-Identifier: MPL-2.0 */

// AArch64 kernel task context switch.
//
// `TaskContext` layout (see `mod.rs`):
//   0x00 : sp
//   0x08 : x19  0x10 : x20  0x18 : x21  0x20 : x22
//   0x28 : x23  0x30 : x24  0x38 : x25  0x40 : x26
//   0x48 : x27  0x50 : x28  0x58 : x29 (fp)
//   0x60 : lr  (return address)

.text

.global context_switch
.type context_switch, @function
context_switch: // (nxt: x0, cur: x1)
    mov     x2, sp
    str     x2,  [x1, #0x00]
    stp     x19, x20, [x1, #0x08]
    stp     x21, x22, [x1, #0x18]
    stp     x23, x24, [x1, #0x28]
    stp     x25, x26, [x1, #0x38]
    stp     x27, x28, [x1, #0x48]
    str     x29, [x1, #0x58]
    str     x30, [x1, #0x60]
    // Fallthrough.

.global first_context_switch
.type first_context_switch, @function
first_context_switch: // (nxt: x0)
    ldr     x2,  [x0, #0x00]
    mov     sp, x2
    ldp     x19, x20, [x0, #0x08]
    ldp     x21, x22, [x0, #0x18]
    ldp     x23, x24, [x0, #0x28]
    ldp     x25, x26, [x0, #0x38]
    ldp     x27, x28, [x0, #0x48]
    ldr     x29, [x0, #0x58]
    ldr     x30, [x0, #0x60]
    ret
.size context_switch, .-context_switch
.size first_context_switch, .-first_context_switch

.global kernel_task_entry_wrapper
.type kernel_task_entry_wrapper, @function
kernel_task_entry_wrapper:
    .cfi_startproc
    .cfi_undefined x30              // mark the return address as end of call stack
    bl      kernel_task_entry
    .cfi_endproc
.size kernel_task_entry_wrapper, .-kernel_task_entry_wrapper
