/* SPDX-License-Identifier: MPL-2.0 */

/*
 * ARM64 exception vector table and trap handling.
 *
 * TrapFrame layout (see trap/trap.rs):
 *   +0x000-0x0F8: GeneralRegs (x0..x30 + sp) = 31 regs * 8 bytes
 *   +0x100:       spsr_el1
 *   +0x108:       elr_el1
 * Total sizeof(TrapFrame) = 0x110
 *
 * For EL0 traps, general.sp holds the user stack pointer (from SP_EL0).
 * For EL1 traps, general.sp holds the pre-exception kernel stack pointer.
 *
 * NOTE: TPIDR_EL0 is NOT saved/restored in exception handlers.
 * Following the upstream Linux model, TPIDR_EL0 is only saved/restored
 * during context switches via ThreadTls::before_schedule/after_schedule.
 * User-space can freely modify TPIDR_EL0 via `msr` without kernel interception.
 */

.text

/* Constants for TrapFrame layout */
.set PT_REGS_SIZE, 0x110
.set S_LR, 0xF0
.set S_SP, 0xF8
.set S_SPSR, 0x100
.set S_ELR, 0x108

/*
 * kernel_entry - Save registers to TrapFrame on exception entry.
 * Parameter: el = exception level (0 = user, 1 = kernel)
 *
 * This macro implements Linux-style parameterized exception entry,
 * with EL0-specific handling for user stack pointer (SP_EL0).
 */
.macro kernel_entry, el:req
    // Allocate TrapFrame space
    sub     sp, sp, #PT_REGS_SIZE

    // Save general registers x0-x29 (pairs)
    stp     x0, x1,   [sp, #16 * 0]
    stp     x2, x3,   [sp, #16 * 1]
    stp     x4, x5,   [sp, #16 * 2]
    stp     x6, x7,   [sp, #16 * 3]
    stp     x8, x9,   [sp, #16 * 4]
    stp     x10, x11, [sp, #16 * 5]
    stp     x12, x13, [sp, #16 * 6]
    stp     x14, x15, [sp, #16 * 7]
    stp     x16, x17, [sp, #16 * 8]
    stp     x18, x19, [sp, #16 * 9]
    stp     x20, x21, [sp, #16 * 10]
    stp     x22, x23, [sp, #16 * 11]
    stp     x24, x25, [sp, #16 * 12]
    stp     x26, x27, [sp, #16 * 13]
    stp     x28, x29, [sp, #16 * 14]

    // Save x30 (link register) at GeneralRegs.x30 field (offset 0xF0)
    str     x30, [sp, #S_LR]

    // Stack pointer: save to GeneralRegs.sp
    .if \el == 0
        mrs     x21, sp_el0              // Read user stack pointer from SP_EL0
        str     x21, [sp, #S_SP]         // Store to GeneralRegs.sp field (0xF8)
    .else
        add     x21, sp, #PT_REGS_SIZE   // Pre-exception kernel stack pointer
        str     x21, [sp, #S_SP]         // Store to GeneralRegs.sp field (0xF8)
    .endif

    // Save exception context (all levels)
    mrs     x22, elr_el1                 // Exception return address
    mrs     x23, spsr_el1                // Saved processor state
    str     x22, [sp, #S_ELR]            // Store to TrapFrame.elr_el1 (0x108)
    str     x23, [sp, #S_SPSR]           // Store to TrapFrame.spsr_el1 (0x100)
.endm

/*
 * kernel_exit - Restore registers from TrapFrame and return.
 * Parameter: el = exception level (0 = user, 1 = kernel)
 *
 * EL0-specific: restore user stack pointer (SP_EL0)
 * EL1: direct kernel return
 */
.macro kernel_exit, el:req
    // Restore exception context
    ldr     x22, [sp, #S_ELR]            // Load TrapFrame.elr_el1 (0x108)
    ldr     x23, [sp, #S_SPSR]           // Load TrapFrame.spsr_el1 (0x100)
    msr     elr_el1, x22                 // Set exception return address
    msr     spsr_el1, x23                // Set saved processor state

    // EL0-specific: restore user stack pointer from GeneralRegs.sp
    .if \el == 0
        ldr     x23, [sp, #S_SP]          // Load GeneralRegs.sp (0xF8)
        msr     sp_el0, x23               // Restore user stack pointer to SP_EL0
    .endif

    // Restore general registers x0-x29 (pairs)
    ldp     x0, x1,   [sp, #16 * 0]
    ldp     x2, x3,   [sp, #16 * 1]
    ldp     x4, x5,   [sp, #16 * 2]
    ldp     x6, x7,   [sp, #16 * 3]
    ldp     x8, x9,   [sp, #16 * 4]
    ldp     x10, x11, [sp, #16 * 5]
    ldp     x12, x13, [sp, #16 * 6]
    ldp     x14, x15, [sp, #16 * 7]
    ldp     x16, x17, [sp, #16 * 8]
    ldp     x18, x19, [sp, #16 * 9]
    ldp     x20, x21, [sp, #16 * 10]
    ldp     x22, x23, [sp, #16 * 11]
    ldp     x24, x25, [sp, #16 * 12]
    ldp     x26, x27, [sp, #16 * 13]
    ldp     x28, x29, [sp, #16 * 14]

    // Restore x30 (link register) from GeneralRegs.x30 field (0xF0)
    ldr     x30, [sp, #S_LR]

    // Restore stack pointer to pre-exception value
    // Note: GeneralRegs.sp field is NOT restored to hardware SP_EL1.
    // The kernel SP is restored by deallocating the TrapFrame.
    add     sp, sp, #PT_REGS_SIZE
.endm

/*
 * Exception vector table (2KB-aligned).
 * Each entry is 128 bytes. The handler sequences exceed 128 bytes,
 * so each slot holds a branch to the full handler code.
 */
    .balign 2048
    .global trap_vectors
trap_vectors:

    /* Current EL with SP0 (EL1t) -- unused */
    .balign 128; b .
    .balign 128; b .
    .balign 128; b .
    .balign 128; b .

    /* Current EL with SPx (EL1h) -- kernel exceptions */
    .balign 128; b    handle_el1h_sync     // Synchronous
    .balign 128; b    handle_el1h_irq      // IRQ
    .balign 128; b .                       // FIQ -- unused
    .balign 128; b .                       // SError -- unused

    /* Lower EL using AArch64 (EL0) -- user exceptions */
    .balign 128; b    handle_el0_sync      // Synchronous (syscalls)
    .balign 128; b    handle_el0_irq       // IRQ
    .balign 128; b .                       // FIQ -- unused
    .balign 128; b .                       // SError -- unused

    /* Lower EL using AArch32 -- unused */
    .balign 128; b .
    .balign 128; b .
    .balign 128; b .
    .balign 128; b .

/*
 * ret_to_kernel - Return path for EL1 (kernel) exceptions.
 */
    .global ret_to_kernel
ret_to_kernel:
    kernel_exit 1                         // EL1 return (no SP_EL0 restore)
    eret                                  // Exception return

/*
 * ret_to_user - Return path for EL0 (user) exceptions.
 * Restores SP_EL0 before returning to user mode.
 */
    .global ret_to_user
ret_to_user:
    kernel_exit 0                         // EL0 return (restore SP_EL0)
    eret                                  // Exception return

/*
 * EL1 exception handlers (kernel mode)
 * Use kernel_entry 1 and ret_to_kernel path
 */
handle_el1h_sync:
    kernel_entry 1                        // Parameter: el=1 (kernel)
    mov     x0, sp                        // Pass TrapFrame pointer
    bl      trap_handler_el1              // Kernel-specific handler
    b       ret_to_kernel                 // Return to kernel

handle_el1h_irq:
    kernel_entry 1                        // Parameter: el=1 (kernel)
    mov     x0, sp                        // Pass TrapFrame pointer
    bl      irq_handler_el1               // Kernel-specific handler
    b       ret_to_kernel                 // Return to kernel

/*
 * EL0 exception handlers (user mode)
 * Use kernel_entry 0 and ret_to_user path
 */
handle_el0_sync:
    kernel_entry 0                        // Parameter: el=0 (user)
    mov     x0, sp                        // Pass TrapFrame pointer
    bl      trap_handler_el0              // User-specific handler
    b       ret_to_user                   // Return to user (restore SP_EL0)

handle_el0_irq:
    kernel_entry 0                        // Parameter: el=0 (user)
    mov     x0, sp                        // Pass TrapFrame pointer
    bl      irq_handler_el0               // User-specific handler
    b       ret_to_user                   // Return to user (restore SP_EL0)

/*
 * User mode entry.
 * Called as: run_user(&mut RawUserContext) with x0 = context ptr.
 * Loads the context, restores user registers, and executes eret.
 * On return from user (via exception), the exception handler saves
 * the user context back.
 */
    .global run_user
run_user:
    // Save kernel callee-saved regs
    sub     sp, sp, #96
    stp     x19, x20, [sp, #0]
    stp     x21, x22, [sp, #16]
    stp     x23, x24, [sp, #32]
    stp     x25, x26, [sp, #48]
    stp     x27, x28, [sp, #64]
    stp     x29, x30, [sp, #80]
    mov     x19, x0                       // Save RawUserContext pointer

    mov     x28, x19                      // Move context ptr to x28 (x28 saved on stack)

    .global run_user_entry
run_user_entry:
    // Set ELR_EL1 and SPSR_EL1
    ldr     x1, [x28, #0x108]             // Load elr_el1 from context
    msr     elr_el1, x1
    ldr     x1, [x28, #0x100]             // Load spsr_el1 from context
    msr     spsr_el1, x1

    // Restore general regs for user mode (skip x0, x27, x28).
    // x28 holds the context pointer and must not be overwritten
    // until the very end. x27 is restored together with x28.
    //
    // Offsets match kernel_entry save layout:
    //   stp xN, xN+1, [sp, #16*K]  =>  offset = K*16
    ldp     x1, x2,   [x28, #0x08]
    ldp     x3, x4,   [x28, #0x18]
    ldp     x5, x6,   [x28, #0x28]
    ldp     x7, x8,   [x28, #0x38]
    ldp     x9, x10,  [x28, #0x48]
    ldp     x11, x12, [x28, #0x58]
    ldp     x13, x14, [x28, #0x68]
    ldp     x15, x16, [x28, #0x78]
    ldp     x17, x18, [x28, #0x88]
    ldp     x19, x20, [x28, #0x98]
    ldp     x21, x22, [x28, #0xA8]
    ldp     x23, x24, [x28, #0xB8]
    ldp     x25, x26, [x28, #0xC8]

    // Restore x30 (LR) and user SP from context
    ldr     x30, [x28, #0xF0]             // Restore x30 (LR)
    ldr     x1,  [x28, #0xF8]             // Load user stack pointer
    msr     sp_el0, x1                    // Set SP_EL0 for user mode

    // Load x0 from context
    ldr     x0,  [x28, #0x00]

    // Restore x27, x28, x29 LAST -- x28 is the base register, so it must
    // be overwritten only after all other loads are complete.
    // ARM64: ldr xN, [xN, #off] is UNPREDICTABLE, so use a temp reg.
    ldr     x27, [x28, #0xD8]             // Load user x27 from offset 0xD8
    ldr     x29, [x28, #0xE8]             // Load user x29 from offset 0xE8
    ldr     x1,  [x28, #0xE0]             // Load user x28 from offset 0xE0 into temp
    mov     x28, x1                       // x28 now holds user value

    eret                                  // Enter user mode

    // Return path: exception handler sets ELR_EL1 to here
    // and SPSR_EL1 to EL1h, so eret returns to kernel mode.
    .global run_user_done
run_user_done:
    // Restore kernel callee-saved regs
    ldp     x19, x20, [sp, #0]
    ldp     x21, x22, [sp, #16]
    ldp     x23, x24, [sp, #32]
    ldp     x25, x26, [sp, #48]
    ldp     x27, x28, [sp, #64]
    ldp     x29, x30, [sp, #80]
    add     sp, sp, #96
    ret                                  // Return to Rust caller