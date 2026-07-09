/* SPDX-License-Identifier: MPL-2.0 */

// AArch64 exception vectors and trap entry/exit.
//
// The kernel `TrapFrame` saved on the stack has this layout (see `trap.rs`):
//   0x000 : x0..x30        (31 * 8 bytes)
//   0x0f8 : sp             (SP at the point of exception)
//   0x100 : elr_el1
//   0x108 : spsr_el1
//   0x110 : esr_el1
// The frame is rounded up to 0x120 bytes for 16-byte stack alignment.

.section .text

// ---- Kernel trap entry -----------------------------------------------------
// Saves a full `TrapFrame`, calls `trap_handler`, restores, and returns.
.macro SAVE_KERNEL_FRAME
    sub     sp, sp, #0x120
    stp     x0,  x1,  [sp, #0x00]
    stp     x2,  x3,  [sp, #0x10]
    stp     x4,  x5,  [sp, #0x20]
    stp     x6,  x7,  [sp, #0x30]
    stp     x8,  x9,  [sp, #0x40]
    stp     x10, x11, [sp, #0x50]
    stp     x12, x13, [sp, #0x60]
    stp     x14, x15, [sp, #0x70]
    stp     x16, x17, [sp, #0x80]
    stp     x18, x19, [sp, #0x90]
    stp     x20, x21, [sp, #0xa0]
    stp     x22, x23, [sp, #0xb0]
    stp     x24, x25, [sp, #0xc0]
    stp     x26, x27, [sp, #0xd0]
    stp     x28, x29, [sp, #0xe0]
    str     x30,      [sp, #0xf0]
    add     x0, sp, #0x120           // SP prior to the exception
    str     x0,       [sp, #0xf8]
    mrs     x0, elr_el1
    mrs     x1, spsr_el1
    mrs     x2, esr_el1
    stp     x0, x1,   [sp, #0x100]
    str     x2,       [sp, #0x110]
.endm

.macro RESTORE_KERNEL_FRAME
    ldp     x0, x1,   [sp, #0x100]
    msr     elr_el1, x0
    msr     spsr_el1, x1
    ldp     x0,  x1,  [sp, #0x00]
    ldp     x2,  x3,  [sp, #0x10]
    ldp     x4,  x5,  [sp, #0x20]
    ldp     x6,  x7,  [sp, #0x30]
    ldp     x8,  x9,  [sp, #0x40]
    ldp     x10, x11, [sp, #0x50]
    ldp     x12, x13, [sp, #0x60]
    ldp     x14, x15, [sp, #0x70]
    ldp     x16, x17, [sp, #0x80]
    ldp     x18, x19, [sp, #0x90]
    ldp     x20, x21, [sp, #0xa0]
    ldp     x22, x23, [sp, #0xb0]
    ldp     x24, x25, [sp, #0xc0]
    ldp     x26, x27, [sp, #0xd0]
    ldp     x28, x29, [sp, #0xe0]
    ldr     x30,      [sp, #0xf0]
    add     sp, sp, #0x120
.endm

// Synchronous exceptions and SError from the kernel.
.global kernel_sync_entry
.type kernel_sync_entry, @function
kernel_sync_entry:
    SAVE_KERNEL_FRAME
    mov     x0, sp                    // &mut TrapFrame
    bl      trap_handler
    RESTORE_KERNEL_FRAME
    eret
.size kernel_sync_entry, .-kernel_sync_entry

// IRQ/FIQ from the kernel.
.global kernel_irq_entry
.type kernel_irq_entry, @function
kernel_irq_entry:
    SAVE_KERNEL_FRAME
    mov     x0, sp                    // &mut TrapFrame
    bl      irq_handler
    RESTORE_KERNEL_FRAME
    eret
.size kernel_irq_entry, .-kernel_irq_entry

// ---- Enter userspace -------------------------------------------------------
// `run_user(regs: *mut RawUserContext)` in x0.
//
// `RawUserContext` layout (see `trap.rs`):
//   0x000 : x0..x30   (x_i at i*8)
//   0x0f8 : sp        (SP_EL0)
//   0x100 : elr       (ELR_EL1, user PC)
//   0x108 : spsr      (SPSR_EL1)
//   0x110 : tpidr     (TPIDR_EL0)
//   0x118 : esr
//   0x120 : far
//   0x128 : trap_kind
//
// The kernel runs on SP_EL1 with SPSel=1, so SP_EL1 is preserved across the EL0
// excursion: `run_user` saves the callee-saved registers and the `regs` pointer
// on the kernel stack, and the user exception vectors find them there.
.global run_user
.type run_user, @function
run_user:
    stp     x19, x20, [sp, #-0x70]!
    stp     x21, x22, [sp, #0x10]
    stp     x23, x24, [sp, #0x20]
    stp     x25, x26, [sp, #0x30]
    stp     x27, x28, [sp, #0x40]
    stp     x29, x30, [sp, #0x50]
    str     x0,       [sp, #0x60]     // save `regs` pointer

    mov     x18, x0                   // x18 = regs base (user x18 restored last)
    ldr     x1, [x18, #0xf8]
    msr     sp_el0, x1
    ldr     x1, [x18, #0x100]
    msr     elr_el1, x1
    ldr     x1, [x18, #0x108]
    msr     spsr_el1, x1
    ldr     x1, [x18, #0x110]
    msr     tpidr_el0, x1

    ldp     x0, x1,   [x18, #0x00]
    ldp     x2, x3,   [x18, #0x10]
    ldp     x4, x5,   [x18, #0x20]
    ldp     x6, x7,   [x18, #0x30]
    ldp     x8, x9,   [x18, #0x40]
    ldp     x10, x11, [x18, #0x50]
    ldp     x12, x13, [x18, #0x60]
    ldp     x14, x15, [x18, #0x70]
    ldp     x16, x17, [x18, #0x80]
    ldp     x19, x20, [x18, #0x98]
    ldp     x21, x22, [x18, #0xa8]
    ldp     x23, x24, [x18, #0xb8]
    ldp     x25, x26, [x18, #0xc8]
    ldp     x27, x28, [x18, #0xd8]
    ldp     x29, x30, [x18, #0xe8]
    ldr     x18,      [x18, #0x90]
    eret
.size run_user, .-run_user

// ---- User exception return -------------------------------------------------
// Saves the user register state into the `RawUserContext`, records the trap
// kind, then restores the kernel context saved by `run_user` and returns to it.
.macro USER_TRAP_RETURN kind
    stp     x0, x1,   [sp, #-0x100]!
    stp     x2, x3,   [sp, #0x10]
    stp     x4, x5,   [sp, #0x20]
    stp     x6, x7,   [sp, #0x30]
    stp     x8, x9,   [sp, #0x40]
    stp     x10, x11, [sp, #0x50]
    stp     x12, x13, [sp, #0x60]
    stp     x14, x15, [sp, #0x70]
    stp     x16, x17, [sp, #0x80]
    stp     x18, x19, [sp, #0x90]
    stp     x20, x21, [sp, #0xa0]
    stp     x22, x23, [sp, #0xb0]
    stp     x24, x25, [sp, #0xc0]
    stp     x26, x27, [sp, #0xd0]
    stp     x28, x29, [sp, #0xe0]
    str     x30,      [sp, #0xf0]

    ldr     x0, [sp, #0x160]          // `regs` pointer from run_user's frame

    ldp     x2, x3, [sp, #0x00]
    stp     x2, x3, [x0, #0x00]
    ldp     x2, x3, [sp, #0x10]
    stp     x2, x3, [x0, #0x10]
    ldp     x2, x3, [sp, #0x20]
    stp     x2, x3, [x0, #0x20]
    ldp     x2, x3, [sp, #0x30]
    stp     x2, x3, [x0, #0x30]
    ldp     x2, x3, [sp, #0x40]
    stp     x2, x3, [x0, #0x40]
    ldp     x2, x3, [sp, #0x50]
    stp     x2, x3, [x0, #0x50]
    ldp     x2, x3, [sp, #0x60]
    stp     x2, x3, [x0, #0x60]
    ldp     x2, x3, [sp, #0x70]
    stp     x2, x3, [x0, #0x70]
    ldp     x2, x3, [sp, #0x80]
    stp     x2, x3, [x0, #0x80]
    ldp     x2, x3, [sp, #0x90]
    stp     x2, x3, [x0, #0x90]
    ldp     x2, x3, [sp, #0xa0]
    stp     x2, x3, [x0, #0xa0]
    ldp     x2, x3, [sp, #0xb0]
    stp     x2, x3, [x0, #0xb0]
    ldp     x2, x3, [sp, #0xc0]
    stp     x2, x3, [x0, #0xc0]
    ldp     x2, x3, [sp, #0xd0]
    stp     x2, x3, [x0, #0xd0]
    ldp     x2, x3, [sp, #0xe0]
    stp     x2, x3, [x0, #0xe0]
    ldr     x2, [sp, #0xf0]
    str     x2, [x0, #0xf0]

    mrs     x2, sp_el0
    str     x2, [x0, #0xf8]
    mrs     x2, elr_el1
    str     x2, [x0, #0x100]
    mrs     x2, spsr_el1
    str     x2, [x0, #0x108]
    mrs     x2, tpidr_el0
    str     x2, [x0, #0x110]
    mrs     x2, esr_el1
    str     x2, [x0, #0x118]
    mrs     x2, far_el1
    str     x2, [x0, #0x120]
    mov     x2, #\kind
    str     x2, [x0, #0x128]

    add     sp, sp, #0x100            // pop the user save area

    ldp     x19, x20, [sp, #0x00]
    ldp     x21, x22, [sp, #0x10]
    ldp     x23, x24, [sp, #0x20]
    ldp     x25, x26, [sp, #0x30]
    ldp     x27, x28, [sp, #0x40]
    ldp     x29, x30, [sp, #0x50]
    add     sp, sp, #0x70
    ret
.endm

.global user_sync_entry
.type user_sync_entry, @function
user_sync_entry:
    USER_TRAP_RETURN 0                // TRAP_KIND_SYNC
.size user_sync_entry, .-user_sync_entry

.global user_irq_entry
.type user_irq_entry, @function
user_irq_entry:
    USER_TRAP_RETURN 1                // TRAP_KIND_IRQ
.size user_irq_entry, .-user_irq_entry

// ---- Exception vector table ------------------------------------------------
// 16 entries, each occupying 0x80 bytes, aligned to 0x800.
.macro VECTOR target
    .balign 0x80
    b       \target
.endm

.section .text
.balign 0x800
.global exception_vector_table
exception_vector_table:
    // Current EL with SP_EL0
    VECTOR kernel_sync_entry          // Synchronous
    VECTOR kernel_irq_entry           // IRQ
    VECTOR kernel_irq_entry           // FIQ
    VECTOR kernel_sync_entry          // SError

    // Current EL with SP_ELx
    VECTOR kernel_sync_entry          // Synchronous
    VECTOR kernel_irq_entry           // IRQ
    VECTOR kernel_irq_entry           // FIQ
    VECTOR kernel_sync_entry          // SError

    // Lower EL, AArch64
    VECTOR user_sync_entry            // Synchronous
    VECTOR user_irq_entry             // IRQ
    VECTOR user_irq_entry             // FIQ
    VECTOR user_sync_entry            // SError

    // Lower EL, AArch32 (unsupported)
    VECTOR kernel_sync_entry          // Synchronous
    VECTOR kernel_irq_entry           // IRQ
    VECTOR kernel_irq_entry           // FIQ
    VECTOR kernel_sync_entry          // SError
