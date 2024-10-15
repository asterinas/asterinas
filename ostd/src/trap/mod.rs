// SPDX-License-Identifier: MPL-2.0

//! Handles trap across kernel and user space.

mod handler;
mod irq;
pub mod softirq;

pub use handler::in_interrupt_context;
pub use softirq::SoftIrqLine;

#[allow(unused_imports)]
pub(crate) use self::handler::{call_irq_callback_functions, IN_INTERRUPT_CONTEXT};
pub use self::irq::{disable_local, DisabledLocalIrqGuard, IrqCallbackFunction, IrqLine};
pub use crate::arch::trap::TrapFrame;
