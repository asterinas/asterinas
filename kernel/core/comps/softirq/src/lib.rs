// SPDX-License-Identifier: MPL-2.0

//! Software interrupt.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::{boxed::Box, sync::Arc};
use core::sync::atomic::{AtomicU8, Ordering};

use aster_util::per_cpu_counter::PerCpuCounter;
use component::{ComponentInitError, init_component};
use ostd::{
    cpu::{CpuId, PinCurrentCpu},
    cpu_local, cpu_local_cell,
    irq::{
        DisabledLocalIrqGuard, disable_local, register_bottom_half_handler_l1,
        register_bottom_half_handler_l2,
    },
    sync::{Waiter, Waker},
    task::{Task, disable_preempt},
};
use spin::Once;

use self::{
    lock::is_softirq_enabled,
    stats::{IRQ_COUNTERS, NR_IRQ_LINES, process_statistic},
};

mod lock;
pub mod softirq_id;
mod stats;
mod taskless;

pub use self::{
    lock::{BottomHalfDisabled, DisableLocalBottomHalfGuard},
    stats::{
        iter_irq_counts_across_all_cpus, iter_softirq_counts_across_all_cpus,
        iter_softirq_counts_on_cpu,
    },
    taskless::Taskless,
};

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
/// // Define an unused softirq ID.
/// const MY_SOFTIRQ_ID: u8 = 4;
/// // Enable the softirq line of this ID.
/// SoftIrqLine::get(MY_SOFTIRQ_ID).enable(|| {
///     // Define the action to take when the softirq with `MY_SOFTIRQ_ID` is raised
///     // ...
/// });
/// // Later on:
/// SoftIrqLine::get(MY_SOFTIRQ_ID).raise(); // This will trigger the registered callback.
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

#[cfg(ktest)]
pub fn init_for_ktest() {
    init().unwrap();
}

static ENABLED_MASK: AtomicU8 = AtomicU8::new(0);

cpu_local_cell! {
    static PENDING_MASK: u8 = 0;
}

cpu_local! {
    static DAEMON_WAKER: Once<Arc<Waker>> = Once::new();
}

/// Processes pending softirqs.
fn process_pending(irq_guard: DisabledLocalIrqGuard, irq_num: u8) -> DisabledLocalIrqGuard {
    process_statistic(irq_num);

    if !is_softirq_enabled() {
        return irq_guard;
    }
    process_all_pending(irq_guard)
}

/// Processes all pending softirqs regardless of whether softirqs are disabled.
///
/// This is called from the context of arbitrary tasks, meaning that we're processing softirqs using
/// the time budget of another task. If not done carefully, this can cause fairness issues between
/// the softirq workload and normal tasks.
///
/// To address this, we defer the work to a background daemon as soon as the maximum number of
/// iterations is reached, or when the scheduler wants to preempt us.
fn process_all_pending(mut irq_guard: DisabledLocalIrqGuard) -> DisabledLocalIrqGuard {
    const SOFTIRQ_RUN_TIMES: u8 = 5;

    let mut run_time = 0;

    loop {
        let action_mask = {
            let pending_mask = PENDING_MASK.load();
            pending_mask & ENABLED_MASK.load(Ordering::Acquire)
        };

        if action_mask == 0 {
            break;
        }

        // We will defer the remaining work in two cases:
        // 1. if we are overloaded, i.e., there are still pending softirqs after `SOFTIRQ_RUN_TIMES`
        //    interactions, or
        // 2. if another task preempts us, where we will want to give it time to run to ensure
        //    fairness.
        if run_time >= SOFTIRQ_RUN_TIMES || Task::need_yield() {
            // If this is `None`, the work will be processed when the daemon thread starts up.
            if let Some(waker) = DAEMON_WAKER.get_with(&irq_guard).get() {
                let _ = waker.wake_up();
            }
            break;
        }
        run_time += 1;

        PENDING_MASK.store(0);

        drop(irq_guard);

        run_callbacks(action_mask);

        irq_guard = disable_local();
    }

    irq_guard
}

/// Runs a loop on the current CPU in a daemon kernel thread that processes softirqs.
///
/// This must be called on a kernel thread bound to a specific CPU. It should be called once on each
/// CPU.
///
/// Usually, softirqs are processed in the bottom half of an IRQ handler. However, if the bottom
/// half is overloaded or causing fairness issues, the work is deferred to a background kernel
/// thread that runs this loop.
pub fn daemon_loop_on_cpu() {
    let (waiter, waker) = Waiter::new_pair();

    // Disable preemption. We should process softirqs in atomic mode anyway.
    let mut preempt_guard = disable_preempt();

    // Register the daemon thread.
    DAEMON_WAKER
        .get_on_cpu(preempt_guard.current_cpu())
        .call_once(|| waker);

    // Before the daemon thread starts, some work may not have been processed in the bottom half.
    let mut should_wait = false;

    loop {
        if should_wait {
            drop(preempt_guard);
            waiter.wait();
            preempt_guard = disable_preempt();

            should_wait = false;
        }

        let action_mask = {
            let _irq_guard = disable_local();

            let mut pending_mask = PENDING_MASK.load();
            pending_mask &= ENABLED_MASK.load(Ordering::Acquire);

            if pending_mask == 0 {
                // There is nothing to do. Wait for some sofirqs to come in and be deferred to the
                // daemon thread.
                should_wait = true;
                continue;
            }

            PENDING_MASK.store(0);

            // Prevent reentrancy in softirq callbacks.
            lock::DISABLE_SOFTIRQ_COUNT.add_assign(1);

            pending_mask
        };

        run_callbacks(action_mask);

        lock::DISABLE_SOFTIRQ_COUNT.sub_assign(1);

        // Allow the scheduler to preempt us to ensure fairness.
        if Task::need_yield() {
            drop(preempt_guard);
            Task::yield_now();
            preempt_guard = disable_preempt();
        }
    }
}

fn run_callbacks(mut action_mask: u8) {
    while action_mask > 0 {
        let action_id = u8::trailing_zeros(action_mask) as u8;

        let softirq_line = SoftIrqLine::get(action_id);
        softirq_line
            .counter
            .get()
            .unwrap()
            // No races because we are either in IRQs or in a daemon thread that should have been
            // bound to a specific CPU.
            .add_on_cpu(CpuId::current_racy(), 1);
        softirq_line.callback.get().unwrap()();

        action_mask &= action_mask - 1;
    }
}
