// SPDX-License-Identifier: MPL-2.0

//! Handles trap across kernel and user space.

mod handler;
mod irq;
pub mod softirq;

pub use handler::in_interrupt_context;
pub use softirq::SoftIrqLine;

pub(crate) use self::handler::call_irq_callback_functions;
pub use self::irq::{disable_local, DisabledLocalIrqGuard, IrqCallbackFunction, IrqLine};
pub use crate::arch::trap::TrapFrame;
