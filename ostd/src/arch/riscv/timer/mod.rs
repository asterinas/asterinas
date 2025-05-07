// SPDX-License-Identifier: MPL-2.0

//! The timer support.

use core::{arch::asm, sync::atomic::Ordering};

use spin::Once;

use crate::{
    arch::boot::DEVICE_TREE,
    cpu::{CpuId, PinCurrentCpu},
    timer::INTERRUPT_CALLBACKS,
    trap,
};

/// The timer frequency (Hz). Here we choose 1000Hz since 1000Hz is easier for unit conversion and
/// convenient for timer. What's more, the frequency cannot be set too high or too low, 1000Hz is
/// a modest choice.
///
/// For system performance reasons, this rate cannot be set too high, otherwise most of the time
/// is spent executing timer code.
pub const TIMER_FREQ: u64 = 1000;

pub(crate) const TIMEBASE_FREQ: Once<u64> = Once::new();
const TIMER_INTERVAL: Once<u64> = Once::new();

pub(super) fn init() {
    TIMEBASE_FREQ.call_once(|| {
        DEVICE_TREE
            .get()
            .unwrap()
            .cpus()
            .next()
            .unwrap()
            .timebase_frequency() as u64
    });
    TIMER_INTERVAL.call_once(|| *TIMEBASE_FREQ.get().unwrap() / TIMER_FREQ);

    enable_timer();
}

pub(super) fn handle_timer_interrupt() {
    let irq_guard = trap::disable_local();
    if irq_guard.current_cpu() == CpuId::bsp() {
        crate::timer::jiffies::ELAPSED.fetch_add(1, Ordering::SeqCst);
    }

    let callbacks_guard = INTERRUPT_CALLBACKS.get_with(&irq_guard);
    for callback in callbacks_guard.borrow().iter() {
        (callback)();
    }
    drop(callbacks_guard);

    setup_timer();
}

fn enable_timer() {
    setup_timer();
    // SAFETY: We enable timer interrupt here.
    unsafe {
        riscv::register::sie::set_stimer();
    }
}

fn setup_timer() {
    if is_sstc_enabled() {
        // SAFETY: We set stimecmp CSR for next timer interrupt.
        unsafe {
            asm!("csrrw {}, stimecmp, {}", out(reg) _, in(reg) get_next_when());
        }
    } else {
        sbi_rt::set_timer(*TIMER_INTERVAL.get().unwrap());
    }
}

fn is_sstc_enabled() -> bool {
    let Some(misa) = riscv::register::misa::read() else {
        return false;
    };
    misa.has_extension('S')
}

fn get_next_when() -> u64 {
    let current = riscv::register::time::read64();
    let interval = *TIMER_INTERVAL.get().unwrap();
    current + interval
}
