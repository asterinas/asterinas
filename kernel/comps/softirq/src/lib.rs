// SPDX-License-Identifier: MPL-2.0

//! Software interrupt.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::boxed::Box;
use core::sync::atomic::{AtomicU8, Ordering};

use aster_util::per_cpu_counter::PerCpuCounter;
use component::{init_component, ComponentInitError};
use lock::is_softirq_enabled;
use ostd::{
    cpu::CpuId,
    cpu_local_cell,
    irq::{
        disable_local, register_bottom_half_handler_l1, register_bottom_half_handler_l2,
        DisabledLocalIrqGuard,
    },
};
use spin::Once;
use stats::IRQ_COUNTERS;
pub use stats::{
    iter_irq_counts_across_all_cpus, iter_softirq_counts_across_all_cpus,
    iter_softirq_counts_on_cpu,
};
mod lock;
pub mod softirq_id;
mod stats;
mod taskless;
pub use lock::{BottomHalfDisabled, DisableLocalBottomHalfGuard};
pub use taskless::Taskless;

use crate::stats::{process_statistic, NR_IRQ_LINES};

/// A representation of a software interrupt (softirq) line.
///
/// # Overview
///
/// Softirq is an interrupt mechanism in the kernel that enables bottom-half processing;
/// they are cheaper to execute compared to regular interrupts because softirqs are less
/// time-critical and thus can be processed in a more flexible manner.
///
/// The `SoftIrqLine` struct encapsulates the data and functionality associated with each
/// softirq line, including an identifier and an associated callback that gets triggered
/// when the softirq is raised.
///
/// The `SoftIrqLine` with the smaller ID has the higher execution priority.
///
/// # Example
///
/// ```
/// // Define an unused softirq id.
/// const MY_SOFTIRQ_ID: u8 = 4;
/// // Enable the softirq line of this id.
/// SoftIrqLine::get(MY_SOFTIRQ_ID).enable(|| {
///     // Define the action to take when the softirq with MY_SOFTIRQ_ID is raised
///     // ...
/// });
/// // Later on:
/// SoftIrqLine::get(MY_SOFTIRQ_ID).raise(); // This will trigger the registered callback
/// ```
pub struct SoftIrqLine {
    id: u8,
    callback: Once<Box<dyn Fn() + 'static + Sync + Send>>,
    counter: Once<PerCpuCounter>,
}

impl SoftIrqLine {
    /// The number of softirq lines.
    const NR_LINES: u8 = 8;

    /// Gets a softirq line.
    ///
    /// The value of `id` must be within `0..NR_LINES`.
    pub fn get(id: u8) -> &'static SoftIrqLine {
        &LINES.get().unwrap()[id as usize]
    }

    const fn new(id: u8) -> Self {
        Self {
            id,
            callback: Once::new(),
            counter: Once::new(),
        }
    }

    /// Gets the ID of this softirq line.
    pub fn id(&self) -> u8 {
        self.id
    }

    /// Raises the softirq, marking it as pending.
    ///
    /// If this line is not enabled yet, the method has no effect.
    pub fn raise(&self) {
        PENDING_MASK.bitor_assign(1 << self.id);
    }

    /// Enables a softirq line by registering its callback.
    ///
    /// # Panics
    ///
    /// Each softirq can only be enabled once. Subsequent calls will panic.
    pub fn enable<F>(&self, callback: F)
    where
        F: Fn() + 'static + Sync + Send,
    {
        assert!(!self.is_enabled());

        self.counter.call_once(PerCpuCounter::new);
        self.callback.call_once(|| Box::new(callback));
        ENABLED_MASK.fetch_or(1 << self.id, Ordering::Release);
    }

    /// Returns whether this softirq line is enabled.
    pub fn is_enabled(&self) -> bool {
        ENABLED_MASK.load(Ordering::Acquire) & (1 << self.id) != 0
    }
}

/// A slice that stores the [`SoftIrqLine`]s, whose ID is equal to its offset in the slice.
static LINES: Once<[SoftIrqLine; SoftIrqLine::NR_LINES as usize]> = Once::new();

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    let lines: [SoftIrqLine; SoftIrqLine::NR_LINES as usize] =
        core::array::from_fn(|i| SoftIrqLine::new(i as u8));
    LINES.call_once(|| lines);

    let interrupt_counter: [PerCpuCounter; NR_IRQ_LINES] =
        core::array::from_fn(|_| PerCpuCounter::new());
    IRQ_COUNTERS.call_once(|| interrupt_counter);

    register_bottom_half_handler_l1(process_pending);
    register_bottom_half_handler_l2(process_statistic);
    taskless::init();
    Ok(())
}

static ENABLED_MASK: AtomicU8 = AtomicU8::new(0);

cpu_local_cell! {
    static PENDING_MASK: u8 = 0;
}

/// Processes pending softirqs.
fn process_pending(irq_guard: DisabledLocalIrqGuard, irq_num: u8) -> DisabledLocalIrqGuard {
    if !is_softirq_enabled() {
        return irq_guard;
    }
    process_statistic(irq_num);
    process_all_pending(irq_guard)
}

/// Processes all pending softirqs regardless of whether softirqs are disabled.
///
/// The processing instructions will iterate for `SOFTIRQ_RUN_TIMES` times. If any softirq
/// is raised during the iteration, it will be processed.
fn process_all_pending(mut irq_guard: DisabledLocalIrqGuard) -> DisabledLocalIrqGuard {
    const SOFTIRQ_RUN_TIMES: u8 = 5;

    for _ in 0..SOFTIRQ_RUN_TIMES {
        let mut action_mask = {
            let pending_mask = PENDING_MASK.load();
            PENDING_MASK.store(0);
            pending_mask & ENABLED_MASK.load(Ordering::Acquire)
        };

        if action_mask == 0 {
            break;
        }

        drop(irq_guard);

        while action_mask > 0 {
            let action_id = u8::trailing_zeros(action_mask) as u8;

            let softirq_line = SoftIrqLine::get(action_id);
            softirq_line
                .counter
                .get()
                .unwrap()
                // No races because we are in IRQs.
                .add_on_cpu(CpuId::current_racy(), 1);
            softirq_line.callback.get().unwrap()();

            action_mask &= action_mask - 1;
        }

        irq_guard = disable_local();
    }

    // TODO: Wake up ksoftirqd if some softirqs are still pending.

    irq_guard
}
