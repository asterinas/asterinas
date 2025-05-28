// SPDX-License-Identifier: MPL-2.0

//! Handles trap across kernel and user space.

mod handler;
pub mod irq;

pub(crate) use handler::call_irq_callback_functions;
pub use handler::{in_interrupt_context, register_bottom_half_handler};
