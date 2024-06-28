// SPDX-License-Identifier: MPL-2.0

//! Handles exceptions (including traps, faults or interrupts) across kernel and user space.

pub mod handler;
mod irq;
pub mod softirq;

pub use softirq::SoftIrqLine;
pub use trapframe::TrapFrame;

pub use self::{
    handler::in_interrupt_context,
    irq::{
        disable_local_irq, enable_local_irq, DisabledLocalIrqGuard, IrqCallbackFunction, IrqLine,
    },
};

pub(crate) fn init() {
    unsafe {
        trapframe::init();
    }
    softirq::init();
}
