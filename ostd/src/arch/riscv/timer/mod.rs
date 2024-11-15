// SPDX-License-Identifier: MPL-2.0

//! The timer support.

use core::sync::atomic::{AtomicU64, Ordering};

use spin::Once;

use crate::{
    arch::{boot::DEVICE_TREE, device::create_device_io_mem, irq::TIMER_IRQ_LINE},
    io_mem::IoMem,
    timer::INTERRUPT_CALLBACKS,
    trap::{self, IrqLine, TrapFrame},
};

/// The timer frequency (Hz). Here we choose 1000Hz since 1000Hz is easier for unit conversion and
/// convenient for timer. What's more, the frequency cannot be set too high or too low, 1000Hz is
/// a modest choice.
///
/// For system performance reasons, this rate cannot be set too high, otherwise most of the time
/// is spent executing timer code.
pub const TIMER_FREQ: u64 = 1000;

pub(crate) static TIMEBASE_FREQ: AtomicU64 = AtomicU64::new(1);
static TIMER_STEP: AtomicU64 = AtomicU64::new(1);
static TIMER_IRQ: Once<IrqLine> = Once::new();

/// [`IoMem`] of goldfish RTC, which will be used by `aster-time`.
pub static GOLDFISH_IO_MEM: Once<IoMem> = Once::new();

pub(super) fn init() {
    init_timer();
    init_rtc();
}

fn init_timer() {
    let timer_freq = DEVICE_TREE
        .get()
        .unwrap()
        .cpus()
        .next()
        .unwrap()
        .timebase_frequency() as u64;
    let timer_step = timer_freq / TIMER_FREQ;
    TIMEBASE_FREQ.store(timer_freq, Ordering::Relaxed);
    TIMER_STEP.store(timer_step, Ordering::Relaxed);

    set_next_timer();
    unsafe {
        riscv::register::sie::set_stimer();
    }

    let mut irq = IrqLine::alloc_specific(TIMER_IRQ_LINE as u8).unwrap();
    irq.on_active(timer_callback);
    TIMER_IRQ.call_once(|| irq);

    log::debug!("Timer initialized with frequency: {timer_freq} Hz, timer step: {timer_step} Hz",);
}

fn set_next_timer() {
    let timer_step = TIMER_STEP.load(Ordering::Relaxed);
    let now = riscv::register::time::read64();
    sbi_rt::set_timer(now + timer_step);
}

pub(crate) fn timer_callback(_: &TrapFrame) {
    crate::timer::jiffies::ELAPSED.fetch_add(1, Ordering::SeqCst);

    let irq_guard = trap::disable_local();
    let callbacks_guard = INTERRUPT_CALLBACKS.get_with(&irq_guard);
    for callback in callbacks_guard.borrow().iter() {
        (callback)();
    }
    drop(callbacks_guard);

    set_next_timer();
}

fn init_rtc() {
    let chosen = DEVICE_TREE.get().unwrap().find_node("/soc/rtc").unwrap();
    if let Some(compatible) = chosen.compatible()
        && compatible.all().any(|c| c == "google,goldfish-rtc")
    {
        let region = chosen.reg().unwrap().next().unwrap();
        let io_mem = unsafe { create_device_io_mem(region.starting_address, region.size.unwrap()) };
        GOLDFISH_IO_MEM.call_once(|| io_mem);
    }
}
