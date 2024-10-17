// SPDX-License-Identifier: MPL-2.0

//! Software interrupt.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::boxed::Box;
use core::sync::atomic::{AtomicU8, Ordering};

use component::{init_component, ComponentInitError};
use ostd::{cpu_local_cell, trap::register_bottom_half_handler};
use spin::Once;

pub mod softirq_id;
mod taskless;

pub use taskless::Taskless;

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
        PENDING_MASK.bitor_assign(1 << self.id);
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

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    let lines: [SoftIrqLine; SoftIrqLine::NR_LINES as usize] =
        core::array::from_fn(|i| SoftIrqLine::new(i as u8));
    LINES.call_once(|| lines);
    register_bottom_half_handler(process_pending);

    taskless::init();
    Ok(())
}

static ENABLED_MASK: AtomicU8 = AtomicU8::new(0);

cpu_local_cell! {
    static PENDING_MASK: u8 = 0;
    static IS_ENABLED: bool = true;
}

/// Enables softirq in current processor.
fn enable_softirq_local() {
    IS_ENABLED.store(true);
}

/// Disables softirq in current processor.
fn disable_softirq_local() {
    IS_ENABLED.store(false);
}

/// Checks whether the softirq is enabled in current processor.
fn is_softirq_enabled() -> bool {
    IS_ENABLED.load()
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

    disable_softirq_local();

    for _i in 0..SOFTIRQ_RUN_TIMES {
        let mut action_mask = {
            let pending_mask = PENDING_MASK.load();
            PENDING_MASK.store(0);
            pending_mask & ENABLED_MASK.load(Ordering::Acquire)
        };

        if action_mask == 0 {
            break;
        }
        while action_mask > 0 {
            let action_id = u8::trailing_zeros(action_mask) as u8;
            SoftIrqLine::get(action_id).callback.get().unwrap()();
            action_mask &= action_mask - 1;
        }
    }

    enable_softirq_local();
}
