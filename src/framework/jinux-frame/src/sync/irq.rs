use super::percpu::PerCpu;
use crate::cpu_local;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use x86_64::instructions::interrupts;

/// Disable all IRQs on the current CPU (i.e., locally).
///
/// This function returns a guard object, which will automatically enable local IRQs again when
/// it is dropped. This function works correctly even when it is called in a _nested_ way.
/// The local IRQs shall only be re-enabled when the most outer guard is dropped.
///
/// This function can play nicely with `SpinLock` as the type uses this function internally.
/// One can invoke this function even after acquiring a spin lock. And the reversed order is also ok.
///
/// # Example
///
/// ``rust
/// use jinux_frame::irq;
///
/// {
///     let _ = irq::disable_local();
///     todo!("do something when irqs are disabled");
/// }
/// ```
#[must_use]
pub fn disable_local() -> DisabledLocalIrqGuard {
    DisabledLocalIrqGuard::new()
}

/// A guard for disabled local IRQs.
pub struct DisabledLocalIrqGuard {
    // Having a private field prevents user from constructing values of this type directly.
    private: (),
}

impl !Send for DisabledLocalIrqGuard {}

cpu_local! {
    static IRQ_OFF_COUNT: IrqInfo = IrqInfo::new();
}

impl DisabledLocalIrqGuard {
    fn new() -> Self {
        IRQ_OFF_COUNT.borrow().inc();
        Self { private: () }
    }
}

impl Drop for DisabledLocalIrqGuard {
    fn drop(&mut self) {
        IRQ_OFF_COUNT.borrow().dec();
    }
}

#[derive(Debug, Default)]
struct IrqInfo {
    // number of interrupt counterpart
    off_num: AtomicU32,
    // interrupt state before calling dec()/inc()
    interrupt_enable: AtomicBool,
}

impl IrqInfo {
    const fn new() -> Self {
        Self {
            off_num: AtomicU32::new(0),
            interrupt_enable: AtomicBool::new(false),
        }
    }

    fn inc(&self) {
        let enabled = interrupts::are_enabled();
        let off_num = self.off_num.load(Ordering::Relaxed);
        if off_num == 0 {
            self.interrupt_enable.store(enabled, Ordering::Relaxed);
        }
        if enabled {
            interrupts::disable();
        }
        self.off_num.fetch_add(1, Ordering::Relaxed);
    }

    fn dec(&self) {
        let off_num = self.off_num.load(Ordering::Relaxed);

        if off_num < 1 {
            // disable_interrupt/inc() should be called before enable_interrupt/dec()
            panic!("The enable_interrupt and disable_interrupt are the counterpart");
        }
        let update_num = self.off_num.fetch_sub(1, Ordering::Relaxed) - 1;
        if update_num == 0 && self.interrupt_enable.load(Ordering::Relaxed) {
            interrupts::enable();
        }
    }
}
