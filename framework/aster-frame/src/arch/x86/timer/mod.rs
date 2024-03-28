// SPDX-License-Identifier: MPL-2.0

pub mod apic;
pub mod hpet;
pub mod pit;

use alloc::{boxed::Box, vec::Vec};
use core::{
    cell::RefCell,
    sync::atomic::{AtomicU64, AtomicU8, Ordering},
    time::Duration,
};

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

pub static TIMER_IRQ_NUM: AtomicU8 = AtomicU8::new(32);
pub static JIFFIES: AtomicU64 = AtomicU64::new(0);

static TIMER_IRQ: Once<IrqLine> = Once::new();

/// Return the `Duration` calculated from the jiffies counts.
pub fn jiffies_as_duration() -> Duration {
    let jiffies = JIFFIES.load(Ordering::Acquire);
    Duration::from_millis(jiffies * 1000 / TIMER_FREQ)
}

pub fn init() {
    if kernel::apic::APIC_INSTANCE.is_completed() {
        // Get the free irq number first. Use `allocate_target_irq` to get the Irq handle after dropping it.
        // Because the function inside `apic::init` will allocate this irq.
        let irq = IrqLine::alloc().unwrap();
        TIMER_IRQ_NUM.store(irq.num(), Ordering::Relaxed);
        drop(irq);
        apic::init();
    } else {
        pit::init(pit::OperatingMode::SquareWaveGenerator);
    };
    let mut timer_irq = IrqLine::alloc_specific(TIMER_IRQ_NUM.load(Ordering::Relaxed)).unwrap();
    timer_irq.on_active(timer_callback);
    TIMER_IRQ.call_once(|| timer_irq);
}

cpu_local! {
    static INTERRUPT_CALLBACKS: RefCell<Vec<Box<dyn Fn() + Sync + Send>>> = RefCell::new(Vec::new());
}

/// Register a function that will be executed during the system timer interruption.
pub fn register_interrupt_callback<F>(func: Box<F>)
where
    F: Fn() + Sync + Send + 'static,
{
    CpuLocal::borrow_with(&INTERRUPT_CALLBACKS, |callbacks| {
        callbacks.borrow_mut().push(func);
    });
}

fn timer_callback(trap_frame: &TrapFrame) {
    let current_jiffies = JIFFIES.fetch_add(1, Ordering::SeqCst);

    CpuLocal::borrow_with(&INTERRUPT_CALLBACKS, |callbacks| {
        for callback in callbacks.borrow().iter() {
            (callback)();
        }
    });

    if APIC_TIMER_CALLBACK.is_completed() {
        APIC_TIMER_CALLBACK.get().unwrap().call(());
    }
}
