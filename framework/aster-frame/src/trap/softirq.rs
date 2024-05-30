// SPDX-License-Identifier: MPL-2.0

//! Software interrupt.

#![allow(unused_variables)]

use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use spin::Once;

use crate::{cpu_local, task::disable_preempt, CpuLocal};

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
        CpuLocal::borrow_with(&PENDING_MASK, |mask| {
            mask.fetch_or(1 << self.id, Ordering::Release);
        });
    }

    /// Enables a softirq line by registering its callback.
    ///
    /// # Panics
    ///
    /// Each softirq can only be enabled once.
    pub fn enable<F>(&self, callback: F)
    where
        F: Fn() + 'static + Sync + Send,
    {
        assert!(!self.is_enabled());

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

pub(super) fn init() {
    let lines: [SoftIrqLine; SoftIrqLine::NR_LINES as usize] =
        array_init::array_init(|i| SoftIrqLine::new(i as u8));
    LINES.call_once(|| lines);
}

static ENABLED_MASK: AtomicU8 = AtomicU8::new(0);

cpu_local! {
    static PENDING_MASK: AtomicU8 = AtomicU8::new(0);
    static IS_ENABLED: AtomicBool = AtomicBool::new(true);
}

/// Enables softirq in current processor.
fn enable_softirq_local() {
    CpuLocal::borrow_with(&IS_ENABLED, |is_enabled| {
        is_enabled.store(true, Ordering::Release)
    })
}

/// Disables softirq in current processor.
fn disable_softirq_local() {
    CpuLocal::borrow_with(&IS_ENABLED, |is_enabled| {
        is_enabled.store(false, Ordering::Release)
    })
}

/// Checks whether the softirq is enabled in current processor.
fn is_softirq_enabled() -> bool {
    CpuLocal::borrow_with(&IS_ENABLED, |is_enabled| is_enabled.load(Ordering::Acquire))
}

/// Processes pending softirqs.
///
/// The processing instructions will iterate for `SOFTIRQ_RUN_TIMES` times. If any softirq
/// is raised during the iteration, it will be processed.
pub(crate) fn process_pending() {
    const SOFTIRQ_RUN_TIMES: u8 = 5;

    if !is_softirq_enabled() {
        return;
    }

    let preempt_guard = disable_preempt();
    disable_softirq_local();

    CpuLocal::borrow_with(&PENDING_MASK, |mask| {
        for i in 0..SOFTIRQ_RUN_TIMES {
            // will not reactive in this handling.
            let mut action_mask = {
                let pending_mask = mask.fetch_and(0, Ordering::Acquire);
                pending_mask & ENABLED_MASK.load(Ordering::Acquire)
            };

            if action_mask == 0 {
                return;
            }
            while action_mask > 0 {
                let action_id = u8::trailing_zeros(action_mask) as u8;
                SoftIrqLine::get(action_id).callback.get().unwrap()();
                action_mask &= action_mask - 1;
            }
        }
    });
    enable_softirq_local();
}
