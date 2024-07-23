// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use trapframe::TrapFrame;

use crate::cpu_local;

cpu_local! {
    static IS_KERNEL_INTERRUPTED: AtomicBool = AtomicBool::new(false);
}

/// Returns true if this function is called within the context of an IRQ handler
/// and the IRQ occurs while the CPU is executing in the kernel mode.
/// Otherwise, it returns false.
pub fn is_kernel_interrupted() -> bool {
    IS_KERNEL_INTERRUPTED.load(Ordering::Acquire)
}

/// Only from kernel
#[no_mangle]
extern "C" fn trap_handler(f: &mut TrapFrame) {
    use riscv::register::scause::{Interrupt::*, Trap};

    IS_KERNEL_INTERRUPTED.store(true, Ordering::Release);
    match riscv::register::scause::read().cause() {
        Trap::Interrupt(_) => todo!(),
        Trap::Exception(e) => {
            let stval = riscv::register::stval::read();
            panic!(
                "Cannot handle kernel cpu exception: {e:?}. stval: {stval:#x}, trapframe: {f:#x?}.",
            );
        }
    }
    IS_KERNEL_INTERRUPTED.store(false, Ordering::Release);
}
