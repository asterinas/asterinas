// SPDX-License-Identifier: MPL-2.0

//! Handles exceptions (including traps, faults or interrupts) across kernel and user space.

mod handler;
pub mod irq;
pub mod softirq;

pub(crate) use handler::user_mode_exception_handler;
pub use trapframe::TrapFrame;

pub use self::handler::in_interrupt_context;

pub(crate) fn init() {
    unsafe {
        trapframe::init();
    }
    softirq::init();
}
