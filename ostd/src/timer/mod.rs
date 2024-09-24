// SPDX-License-Identifier: MPL-2.0

//! The timer support.

pub(crate) mod jiffies;

use alloc::{boxed::Box, vec::Vec};
use core::cell::RefCell;

pub use jiffies::Jiffies;

use crate::{cpu_local, trap};

type InterruptCallback = Box<dyn Fn() + Sync + Send>;

cpu_local! {
    pub(crate) static INTERRUPT_CALLBACKS: RefCell<Vec<InterruptCallback>> = RefCell::new(Vec::new());
}

/// Register a function that will be executed during the system timer interruption.
pub fn register_callback<F>(func: F)
where
    F: Fn() + Sync + Send + 'static,
{
    let irq_guard = trap::disable_local();
    INTERRUPT_CALLBACKS
        .get_with(&irq_guard)
        .borrow_mut()
        .push(Box::new(func));
}
