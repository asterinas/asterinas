// SPDX-License-Identifier: MPL-2.0

//! The timer support.

use core::{
    arch::asm,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    arch::{self, boot::DEVICE_TREE},
    cpu::{extension::IsaExtensions, CpuId, PinCurrentCpu},
    timer::INTERRUPT_CALLBACKS,
    trap,
};

/// The timer frequency (Hz). Here we choose 1000Hz since 1000Hz is easier for
/// unit conversion and convenient for timer. What's more, the frequency cannot
/// be set too high or too low, 1000Hz is a modest choice.
///
/// For system performance reasons, this rate cannot be set too high, otherwise
/// most of the time is spent executing timer code.
pub const TIMER_FREQ: u64 = 1000;

static TIMEBASE_FREQ: AtomicU64 = AtomicU64::new(0);
static TIMER_INTERVAL: AtomicU64 = AtomicU64::new(0);

/// Initializes the timer module.
///
/// # Safety
///
/// This function is safe to call on the following conditions:
/// 1. It is called once and at most once at a proper timing in the boot context.
/// 2. It is called before any other public functions of this module is called.
pub(super) unsafe fn init() {
    TIMEBASE_FREQ.store(
        DEVICE_TREE
            .get()
            .unwrap()
            .cpus()
            .next()
            .unwrap()
            .timebase_frequency() as u64,
        Ordering::Relaxed,
    );
    TIMER_INTERVAL.store(
        TIMEBASE_FREQ.load(Ordering::Relaxed) / TIMER_FREQ,
        Ordering::Relaxed,
    );

    if is_sstc_enabled() {
        // SAFETY: Mutating the static variable `SET_NEXT_TIMER_FN` is safe here
        // because we ensure that it is only modified during the initialization
        // phase of the timer.
        unsafe {
            SET_NEXT_TIMER_FN = set_next_timer_sstc;
        }
    }
    set_next_timer();
    // SAFETY: Accessing the `sie` CSR to enable the timer interrupt is safe
    // here because this function is only called during timer initialization,
    // and we ensure that only the timer interrupt bit is set without affecting
    // other interrupt sources.
    unsafe {
        riscv::register::sie::set_stimer();
    }
}

pub(super) fn handle_timer_interrupt() {
    let irq_guard = trap::irq::disable_local();
    if irq_guard.current_cpu() == CpuId::bsp() {
        crate::timer::jiffies::ELAPSED.fetch_add(1, Ordering::Relaxed);
    }

    let callbacks_guard = INTERRUPT_CALLBACKS.get_with(&irq_guard);
    for callback in callbacks_guard.borrow().iter() {
        (callback)();
    }
    drop(callbacks_guard);

    set_next_timer();
}

fn set_next_timer() {
    // SAFETY: Calling the `SET_NEXT_TIMER_FN` function pointer is safe here
    // because we ensure that it is set to a valid function during the timer
    // initialization, and we never modify it after that.
    unsafe {
        SET_NEXT_TIMER_FN();
    }
}

static mut SET_NEXT_TIMER_FN: fn() = set_next_timer_sbi;

fn set_next_timer_sbi() {
    sbi_rt::set_timer(TIMER_INTERVAL.load(Ordering::Relaxed));
}

fn set_next_timer_sstc() {
    // SAFETY: Setting the next timer using the `stimecmp` CSR is safe here
    // because we are using the `stimecmp` CSR to set the next timer interrupt
    // only when we're handling a timer interrupt, which is a standard operation
    // specified by RISC-V SSTC extension.
    unsafe {
        asm!("csrrw {}, stimecmp, {}", out(reg) _, in(reg) get_next_when());
    }
}

fn is_sstc_enabled() -> bool {
    arch::cpu::extension::has_extensions(IsaExtensions::SSTC)
}

fn get_next_when() -> u64 {
    let current = riscv::register::time::read64();
    let interval = TIMER_INTERVAL.load(Ordering::Relaxed);
    current + interval
}

pub(crate) fn get_timebase_freq() -> u64 {
    TIMEBASE_FREQ.load(Ordering::Relaxed)
}
