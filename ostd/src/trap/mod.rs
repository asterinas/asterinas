// SPDX-License-Identifier: MPL-2.0

//! Handles trap across kernel and user space.

#![cfg_attr(target_arch = "riscv64", expect(unused_imports))]

mod handler;
pub mod irq;

pub(crate) use handler::call_irq_callback_functions;
pub use handler::{
    get_interrupt_stats, get_total_interrupts, in_interrupt_context, register_bottom_half_handler,
};
