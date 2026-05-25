// SPDX-License-Identifier: MPL-2.0

//! Handles trap.

#[expect(clippy::module_inception)]
mod trap;

use spin::Once;
pub(super) use trap::RawUserContext;
pub use trap::TrapFrame;

use crate::{
    arch::cpu::context::CpuException, cpu::PrivilegeLevel, ex_table::ExTable,
    irq::call_irq_callback_functions, mm::MAX_USERSPACE_VADDR,
};

/// Initializes trap handling on ARM64.
///
/// # Safety
///
/// On the current CPU, this function must be called
/// - only once and
/// - before any trap can occur.
pub(crate) unsafe fn init_on_cpu() {
    // SAFETY: The caller ensures the safety conditions.
    unsafe {
        trap::init_on_cpu();
    }
}

/// The return-to-kernel label in run_user. When trap_handler wants to
/// return from run_user (for syscall processing), it sets the trap frame's
/// ELR_EL1 to this address and SPSR_EL1 to EL1h, so that eret returns
/// to run_user_done which restores callee-saved regs and returns from run_user.
fn run_user_done_addr() -> usize {
    trap::run_user_done as *const () as usize
}

/// Handle traps from kernel mode (EL1).
/// Called from assembly handle_el1h_sync with kernel_entry 1.
// SAFETY: The name does not collide with other symbols.
#[unsafe(no_mangle)]
unsafe extern "C" fn trap_handler_el1(f: &mut TrapFrame) {
    let esr: u64;
    // SAFETY: Reading ESR_EL1 is always safe.
    unsafe { core::arch::asm!("mrs {0}, esr_el1", out(reg) esr) };

    // Exception Class (EC) is in bits [31:26] of ESR_EL1.
    let ec = (esr >> 26) as u8 & 0x3F;

    match ec {
        // EC = 0x21: Instruction Abort from same EL (kernel page fault)
        // EC = 0x25: Data Abort from same EL (kernel page fault)
        0x21 | 0x25 => {
            let far = read_far() as usize;

            // If the fault address is in user space, try to handle it as a
            // user-space page fault first (e.g., demand allocation). This
            // matches x86's behavior where kernel accesses to unmapped
            // user-space pages trigger demand paging rather than panicking.
            if far < MAX_USERSPACE_VADDR {
                let handler = USER_PAGE_FAULT_HANDLER.get();
                if let Some(handler) = handler {
                    // Derive the exception type from ESR.
                    let exception = if ec == 0x21 {
                        CpuException::InstructionPageFault(far)
                    } else if esr & (1 << 6) != 0 {
                        CpuException::StorePageFault(far)
                    } else {
                        CpuException::LoadPageFault(far)
                    };

                    if handler(&exception).is_ok() {
                        return;
                    }
                }
            }

            // If demand paging failed or the fault is in kernel space,
            // try the exception table: recover to a predefined handler
            // if the faulting instruction has one.
            if let Some(recovery_addr) = ExTable::find_recovery_inst_addr(f.elr_el1) {
                f.elr_el1 = recovery_addr;
                return;
            }

            panic!(
                "Unhandled kernel page fault: \
                 ESR_EL1={:#018x}, FAR_EL1={:#018x}, ELR_EL1={:#018x}",
                esr, far, f.elr_el1,
            );
        }
        // EC = 0x07: Trapped SIMD/FP access (FPU not enabled)
        0x07 => {
            // SAFETY: CPACR_EL1 writes are always safe.
            unsafe {
                core::arch::asm!(
                    "mrs x9, cpacr_el1",
                    "orr x9, x9, #(3 << 20)",
                    "msr cpacr_el1, x9",
                    "isb",
                    out("x9") _,
                );
            }
        }
        // EC = 0x3C: BRK instruction (debug breakpoint)
        0x3C => {
            // BRK is used for debugging
        }
        // Other: unknown exception
        _ => {
            panic!(
                "Unhandled kernel trap: ESR_EL1={:#018x} EC={}, ELR_EL1={:#018x}",
                esr, ec, f.elr_el1
            );
        }
    }
}

/// Handle traps from user mode (EL0).
/// Called from assembly handle_el0_sync with kernel_entry 0.
/// Automatically retrieves user context from CURRENT_USER_CTX.
// SAFETY: The name does not collide with other symbols.
#[unsafe(no_mangle)]
unsafe extern "C" fn trap_handler_el0(f: &mut TrapFrame) {
    let esr: u64;
    // SAFETY: Reading ESR_EL1 is always safe.
    unsafe { core::arch::asm!("mrs {0}, esr_el1", out(reg) esr) };

    // Exception Class (EC) is in bits [31:26] of ESR_EL1.
    let ec = (esr >> 26) as u8 & 0x3F;

    // Preserve IRQ state before the exception, matching x86_64/riscv64.
    // SPSR_EL1 bit 7 (I) = 0 means IRQs were enabled in the interrupted context.
    let was_irq_enabled = f.spsr_el1 & (1 << 7) == 0;

    // Instruction Abort from lower EL (user)
    if ec == 0x20 {
        if was_irq_enabled {
            crate::arch::irq::enable_local();
        }
        let exception = CpuException::InstructionPageFault(read_far() as usize);
        handle_user_page_fault(f, &exception);
        // Disable IRQs before ret_to_user: arm64's eret sequence (msr elr; msr spsr; eret)
        // is non-atomic. A timer IRQ between msr spsr and eret leaks the TrapFrame
        // (add sp,#0x118 is skipped). Matching x86_64's disable_local_if pattern.
        if was_irq_enabled {
            crate::arch::irq::disable_local();
        }
        return;
    }
    // Data Abort from lower EL (user): WnR bit (ESR_EL1[6]) distinguishes load/store
    if ec == 0x24 {
        if was_irq_enabled {
            crate::arch::irq::enable_local();
        }
        let far = read_far() as usize;
        let exception = if esr & (1 << 6) != 0 {
            CpuException::StorePageFault(far)
        } else {
            CpuException::LoadPageFault(far)
        };
        handle_user_page_fault(f, &exception);
        if was_irq_enabled {
            crate::arch::irq::disable_local();
        }
        return;
    }

    match ec {
        // EC = 0x15: SVC (system call) from AArch64
        0x15 => {
            // Get user context from CURRENT_USER_CTX
            let ctx_ptr = trap::CURRENT_USER_CTX.load();

            if ctx_ptr != 0 {
                // User syscall: save context to RawUserContext and return from run_user.
                let raw_ctx = unsafe { &mut *(ctx_ptr as *mut RawUserContext) };
                raw_ctx.general = f.general;
                raw_ctx.elr_el1 = f.elr_el1;
                raw_ctx.spsr_el1 = f.spsr_el1;
                // Set up return to run_user_done in kernel mode.
                f.elr_el1 = run_user_done_addr();
                // SPSR_EL1: EL1h mode (M=0b0101), all interrupts masked.
                f.spsr_el1 = 0x3C5;
            } else {
                // No user context: ELR_EL1 (set by hardware) already points
                // past SVC. Return ENOSYS without further advancing.
                f.general.x0 = usize::MAX; // -ENOSYS
            }
        }
        // EC = 0x07: Trapped SIMD/FP access (FPU not enabled)
        0x07 => {
            // Enable FP/ASIMD and retry the faulting instruction.
            // SAFETY: CPACR_EL1 writes are always safe.
            unsafe {
                core::arch::asm!(
                    "mrs x9, cpacr_el1",
                    "orr x9, x9, #(3 << 20)",
                    "msr cpacr_el1, x9",
                    "isb",
                    out("x9") _,
                );
            }
        }
        // EC = 0x3C: BRK instruction (debug breakpoint) from AArch64
        0x3C => {
            // BRK is used for debugging; on user side it generates SIGTRAP.
        }
        // Other: unknown exception
        _ => {
            panic!(
                "Unhandled user trap: ESR_EL1={:#018x} EC={}, ELR_EL1={:#018x}",
                esr, ec, f.elr_el1
            );
        }
    }
}

/// Handle IRQ interrupts from kernel mode (EL1).
/// Called from assembly handle_el1h_irq with kernel_entry 1.
// SAFETY: The name does not collide with other symbols.
#[unsafe(no_mangle)]
unsafe extern "C" fn irq_handler_el1(f: &mut TrapFrame) {
    while let Some(hw_irq) = crate::arch::irq::chip::IrqChip::claim_interrupt() {
        call_irq_callback_functions(f, &hw_irq, PrivilegeLevel::Kernel);
    }
}

/// Handle IRQ interrupts from user mode (EL0).
/// Called from assembly handle_el0_irq with kernel_entry 0.
// SAFETY: The name does not collide with other symbols.
#[unsafe(no_mangle)]
unsafe extern "C" fn irq_handler_el0(f: &mut TrapFrame) {
    while let Some(hw_irq) = crate::arch::irq::chip::IrqChip::claim_interrupt() {
        call_irq_callback_functions(f, &hw_irq, PrivilegeLevel::User);
    }
}

/// Reads FAR_EL1 (Fault Address Register).
fn read_far() -> u64 {
    let far: u64;
    // SAFETY: Reading FAR_EL1 is always safe.
    unsafe { core::arch::asm!("mrs {0}, far_el1", out(reg) far) };
    far
}

#[expect(clippy::type_complexity)]
static USER_PAGE_FAULT_HANDLER: Once<fn(&CpuException) -> Result<(), ()>> = Once::new();

/// Injects a custom handler for page faults that occur in the kernel and
/// are caused by user-space address.
pub fn inject_user_page_fault_handler(handler: fn(info: &CpuException) -> Result<(), ()>) {
    USER_PAGE_FAULT_HANDLER.call_once(|| handler);
}

fn handle_user_page_fault(f: &mut TrapFrame, exception: &CpuException) {
    let handler = USER_PAGE_FAULT_HANDLER
        .get()
        .expect("Page fault handler is missing");

    let res = handler(exception);
    if res.is_ok() {
        return;
    }

    // Use the exception table to recover from kernel-mode faults.
    if let Some(addr) = ExTable::find_recovery_inst_addr(f.elr_el1) {
        f.elr_el1 = addr;
        return;
    }

    // For user-mode faults that can't be handled inline: return from run_user
    // so UserContext::execute can deliver a signal.
    let ctx_ptr = trap::CURRENT_USER_CTX.load();

    if ctx_ptr != 0 && f.elr_el1 < MAX_USERSPACE_VADDR {
        let raw_ctx = unsafe { &mut *(ctx_ptr as *mut RawUserContext) };
        raw_ctx.general = f.general;
        raw_ctx.elr_el1 = f.elr_el1;
        raw_ctx.spsr_el1 = f.spsr_el1;
        f.elr_el1 = run_user_done_addr();
        f.spsr_el1 = 0x3C5;
    }
}
