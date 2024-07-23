// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, vec::Vec};
use core::{
    cell::RefCell,
    sync::atomic::{AtomicU64, Ordering},
};

pub use jiffies::Jiffies;
use spin::Once;

use crate::{arch::boot::DEVICE_TREE, cpu_local, io_mem::IoMem};

mod jiffies;

/// The timer frequency (Hz). Here we choose 1000Hz since 1000Hz is easier for unit conversion and
/// convenient for timer. What's more, the frequency cannot be set too high or too low, 1000Hz is
/// a modest choice.
///
/// For system performance reasons, this rate cannot be set too high, otherwise most of the time
/// is spent executing timer code.
///
/// Due to hardware limitations, this value cannot be set too low; for example, PIT cannot accept
/// frequencies lower than 19Hz = 1193182 / 65536 (Timer rate / Divider)
pub const TIMER_FREQ: u64 = 1000;

pub static TIMEBASE_FREQ: AtomicU64 = AtomicU64::new(1);
pub static TIMER_STEP: AtomicU64 = AtomicU64::new(1);
pub static CURRENT_TIME: AtomicU64 = AtomicU64::new(0);

pub static GOLDFISH_IO_MEM: Once<IoMem> = Once::new();

pub fn init() {
    let timer_freq = DEVICE_TREE
        .get()
        .unwrap()
        .cpus()
        .next()
        .unwrap()
        .timebase_frequency() as u64;
    TIMEBASE_FREQ.store(timer_freq, Ordering::Relaxed);
    TIMER_STEP.store(timer_freq / TIMER_FREQ, Ordering::Relaxed);
    log::debug!(
        "Timer initialized with frequency: {} Hz, timer step: {} Hz",
        timer_freq,
        TIMER_STEP.load(Ordering::Relaxed)
    );
    set_next_timer();

    let chosen = DEVICE_TREE.get().unwrap().find_node("/soc/rtc").unwrap();
    let region = chosen.reg().unwrap().next().unwrap();
    let io_mem = unsafe {
        IoMem::new(
            (region.starting_address as usize)
                ..(region.starting_address as usize) + region.size.unwrap(),
        )
    };
    GOLDFISH_IO_MEM.call_once(|| io_mem);
}

fn set_next_timer() {
    // TODO: fix
    sbi_rt::set_timer(TIMER_STEP.load(Ordering::Relaxed));
}

cpu_local! {
    static INTERRUPT_CALLBACKS: RefCell<Vec<Box<dyn Fn() + Sync + Send>>> = RefCell::new(Vec::new());
}

/// Register a function that will be executed during the system timer interruption.
pub fn register_callback<F>(func: F)
where
    F: Fn() + Sync + Send + 'static,
{
    INTERRUPT_CALLBACKS
        .borrow_irq_disabled()
        .borrow_mut()
        .push(Box::new(func));
}

pub fn timer_callback() {
    jiffies::ELAPSED.fetch_add(1, Ordering::SeqCst);

    for callback in INTERRUPT_CALLBACKS.borrow_irq_disabled().borrow().iter() {
        (callback)();
    }

    set_next_timer();
}
