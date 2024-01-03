// SPDX-License-Identifier: MPL-2.0

mod handler;
mod irq;

pub(crate) use self::handler::call_irq_callback_functions;
pub use self::irq::{disable_local, DisabledLocalIrqGuard, IrqCallbackFunction, IrqLine};
pub use trapframe::TrapFrame;

pub(crate) fn init() {
    unsafe {
        trapframe::init();
    }
}
