// SPDX-License-Identifier: MPL-2.0

//! Handles trap.

#[expect(clippy::module_inception)]
mod trap;

pub(super) use trap::RawUserContext;
pub use trap::TrapFrame;

use crate::arch::{
    cpu::context::{CpuException, CpuTrap},
    irq::{PSTATE_I, disable_local, enable_local},
};

/// Initializes interrupt handling on ARM.
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

/// Handle traps (only from kernel).
// SAFETY: The name does not collide with other symbols.
#[unsafe(no_mangle)]
extern "C" fn trap_handler(f: &mut TrapFrame) {
    fn enable_local_if(cond: bool) {
        if cond {
            enable_local();
        }
    }

    fn disable_local_if(cond: bool) {
        if cond {
            disable_local();
        }
    }

    // The IRQ state before trapping. We need to ensure that the IRQ state
    // during exception handling is consistent with the state before the trap.
    let was_irq_enabled = f.spsr & PSTATE_I == 0;

    let trap = CpuTrap::new(f.trap_num);
    match trap {
        Some(CpuTrap::Exception(data_abort @ CpuException::DataAbort { address, .. })) => {
            enable_local_if(was_irq_enabled);
            crate::mm::fault::handle_user_page_fault(f, &data_abort, address);
            disable_local_if(was_irq_enabled);
        }
        _ => panic!(
            "Cannot handle kernel CPU exception: {:?}; trapframe: {:#?}",
            trap, f
        ),
    }
}
