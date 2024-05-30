// SPDX-License-Identifier: MPL-2.0

//! The timer support.

mod apic;
mod hpet;
mod jiffies;
pub(crate) mod pit;

use alloc::{boxed::Box, vec::Vec};
use core::{cell::RefCell, sync::atomic::Ordering};

pub use jiffies::Jiffies;
use spin::Once;
use trapframe::TrapFrame;

use self::apic::APIC_TIMER_CALLBACK;
use crate::{arch::x86::kernel, cpu_local, trap::IrqLine, CpuLocal};

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

static TIMER_IRQ: Once<IrqLine> = Once::new();

pub(super) fn init() {
    /// In PIT mode, channel 0 is connected directly to IRQ0, which is
    /// the `IrqLine` with the `irq_num` 32 (0-31 `IrqLine`s are reserved).
    ///
    /// Ref: https://wiki.osdev.org/Programmable_Interval_Timer#Outputs.
    const PIT_MODE_TIMER_IRQ_NUM: u8 = 32;

    let mut timer_irq = if kernel::apic::APIC_INSTANCE.is_completed() {
        apic::init()
    } else {
        pit::init(pit::OperatingMode::SquareWaveGenerator);
        IrqLine::alloc_specific(PIT_MODE_TIMER_IRQ_NUM).unwrap()
    };

    timer_irq.on_active(timer_callback);
    TIMER_IRQ.call_once(|| timer_irq);
}

cpu_local! {
    static INTERRUPT_CALLBACKS: RefCell<Vec<Box<dyn Fn() + Sync + Send>>> = RefCell::new(Vec::new());
}

/// Registers a function that will be executed during the system timer interruption.
pub fn register_callback<F>(func: F)
where
    F: Fn() + Sync + Send + 'static,
{
    CpuLocal::borrow_with(&INTERRUPT_CALLBACKS, |callbacks| {
        callbacks.borrow_mut().push(Box::new(func));
    });
}

fn timer_callback(_: &TrapFrame) {
    jiffies::ELAPSED.fetch_add(1, Ordering::SeqCst);

    CpuLocal::borrow_with(&INTERRUPT_CALLBACKS, |callbacks| {
        for callback in callbacks.borrow().iter() {
            (callback)();
        }
    });

    if APIC_TIMER_CALLBACK.is_completed() {
        APIC_TIMER_CALLBACK.get().unwrap().call(());
    }
}
