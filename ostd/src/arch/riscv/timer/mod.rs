// SPDX-License-Identifier: MPL-2.0

//! The timer support.

use core::sync::atomic::{AtomicU64, Ordering};

use riscv::register::{sie, time};

use crate::{
    arch::boot::DEVICE_TREE,
    timer::INTERRUPT_CALLBACKS,
    trap::{self, IN_INTERRUPT_CONTEXT},
};

/// The timer frequency (Hz). Here we choose 1000Hz since 1000Hz is easier for unit conversion and
/// convenient for timer. What's more, the frequency cannot be set too high or too low, 1000Hz is
/// a modest choice.
///
/// For system performance reasons, this rate cannot be set too high, otherwise most of the time
/// is spent executing timer code.
pub const TIMER_FREQ: u64 = 1000;

pub(crate) static TIMEBASE_FREQ: AtomicU64 = AtomicU64::new(1);

pub(super) fn init() {
    let timer_freq = DEVICE_TREE
        .get()
        .unwrap()
        .cpus()
        .next()
        .unwrap()
        .timebase_frequency() as u64;
    TIMEBASE_FREQ.store(timer_freq, Ordering::Relaxed);

    unsafe {
        sie::set_stimer();
        set_next_tick();
    }
}

pub(crate) fn timer_callback() {
    IN_INTERRUPT_CONTEXT.store(true);

    crate::timer::jiffies::ELAPSED.fetch_add(1, Ordering::SeqCst);

    let irq_guard = trap::disable_local();
    let callbacks_guard = INTERRUPT_CALLBACKS.get_with(&irq_guard);
    for callback in callbacks_guard.borrow().iter() {
        (callback)();
    }

    set_next_tick();
    IN_INTERRUPT_CONTEXT.store(false);
}

fn set_next_tick() {
    let next_tick = time::read64() + TIMEBASE_FREQ.load(Ordering::Relaxed) / TIMER_FREQ;
    sbi_rt::set_timer(next_tick);
}
