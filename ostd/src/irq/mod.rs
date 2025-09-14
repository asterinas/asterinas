// SPDX-License-Identifier: MPL-2.0

//! Interrupt ReQuest (IRQ) handling.

mod guard;
mod handler;
mod line;

pub use guard::{disable_local, DisabledLocalIrqGuard};
pub(crate) use handler::call_irq_callback_functions;
pub use handler::{in_interrupt_context, register_bottom_half_handler};
pub use line::{IrqCallbackFunction, IrqLine};
