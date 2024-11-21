// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, vec, vec::Vec};

use aster_softirq::{softirq_id::TIMER_SOFTIRQ_ID, SoftIrqLine};
use ostd::{sync::LazyRcu, timer};

#[allow(clippy::type_complexity)]
#[allow(clippy::box_collection)]
static TIMER_SOFTIRQ_CALLBACKS: LazyRcu<Box<Vec<fn()>>> = LazyRcu::new_uninit();

pub(super) fn init() {
    SoftIrqLine::get(TIMER_SOFTIRQ_ID).enable(timer_softirq_handler);

    timer::register_callback(|| {
        SoftIrqLine::get(TIMER_SOFTIRQ_ID).raise();
    });
}

/// Registers a function that will be executed during timer softirq.
pub(super) fn register_callback(func: fn()) {
    loop {
        let callbacks = TIMER_SOFTIRQ_CALLBACKS.read_maybe_uninit();
        match callbacks.check() {
            // Initialized, copy the vector, push the function and update.
            Ok(callbacks) => {
                let mut callbacks_cloned = (*callbacks).clone();
                callbacks_cloned.push(func);
                if callbacks
                    .try_compare_update(Box::new(callbacks_cloned))
                    .is_ok()
                {
                    break;
                }
            }
            // Uninitialized, initialize it.
            Err(callbacks) => {
                if callbacks.try_compare_update(Box::new(vec![func])).is_ok() {
                    break;
                }
            }
        }
        // Contention on initialization or pushing, retry.
        core::hint::spin_loop();
    }
}

fn timer_softirq_handler() {
    let callbacks = TIMER_SOFTIRQ_CALLBACKS.read_maybe_uninit();
    if let Ok(callbacks) = callbacks.check() {
        for callback in callbacks.iter() {
            (callback)();
        }
    }
}
