// SPDX-License-Identifier: MPL-2.0

//! The timer support.

use core::sync::atomic::{AtomicU64, Ordering};

use spin::Once;

use crate::{arch::boot::DEVICE_TREE, io_mem::IoMem};

/// The timer frequency (Hz). Here we choose 1000Hz since 1000Hz is easier for unit conversion and
/// convenient for timer. What's more, the frequency cannot be set too high or too low, 1000Hz is
/// a modest choice.
///
/// For system performance reasons, this rate cannot be set too high, otherwise most of the time
/// is spent executing timer code.
pub const TIMER_FREQ: u64 = 1000;

pub(crate) static TIMEBASE_FREQ: AtomicU64 = AtomicU64::new(1);

/// [`IoMem`] of goldfish RTC, which will be used by `aster-time`.
pub static GOLDFISH_IO_MEM: Once<IoMem> = Once::new();

pub(super) fn init() {
    let timer_freq = DEVICE_TREE
        .get()
        .unwrap()
        .cpus()
        .next()
        .unwrap()
        .timebase_frequency() as u64;
    TIMEBASE_FREQ.store(timer_freq, Ordering::Relaxed);

    let chosen = DEVICE_TREE.get().unwrap().find_node("/soc/rtc").unwrap();
    if let Some(compatible) = chosen.compatible()
        && compatible.all().any(|c| c == "google,goldfish-rtc")
    {
        let region = chosen.reg().unwrap().next().unwrap();
        let io_mem = unsafe {
            IoMem::new(
                (region.starting_address as usize)
                    ..(region.starting_address as usize) + region.size.unwrap(),
            )
        };
        GOLDFISH_IO_MEM.call_once(|| io_mem);
    }
}
