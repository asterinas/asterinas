// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, vec::Vec};

use ostd::{sync::RwLock, timer, trap::SoftIrqLine};

use crate::softirq_id::TIMER_SOFTIRQ_ID;

static TIMER_SOFTIRQ_CALLBACKS: RwLock<Vec<Box<dyn Fn() + Sync + Send>>> = RwLock::new(Vec::new());

pub(super) fn init() {
    SoftIrqLine::get(TIMER_SOFTIRQ_ID).enable(timer_softirq_handler);

    timer::register_callback(|| {
        SoftIrqLine::get(TIMER_SOFTIRQ_ID).raise();
    });
}

/// Registers a function that will be executed during timer softirq.
pub(super) fn register_callback<F>(func: F)
where
    F: Fn() + Sync + Send + 'static,
{
    TIMER_SOFTIRQ_CALLBACKS
        .write_irq_disabled()
        .push(Box::new(func));
}

fn timer_softirq_handler() {
    let callbacks = TIMER_SOFTIRQ_CALLBACKS.read_irq_disabled();
    for callback in callbacks.iter() {
        (callback)();
    }
}
