// SPDX-License-Identifier: MPL-2.0

//! Defines the used IDs of software interrupt (softirq) lines.

/// The corresponding softirq line is used to schedule urgent taskless jobs.
pub const TASKLESS_URGENT_SOFTIRQ_ID: u8 = 0;

/// The corresponding softirq line is used to manage timers and handle
/// time-related jobs.
pub const TIMER_SOFTIRQ_ID: u8 = 1;

/// The corresponding softirq line is used to schedule general taskless jobs.
pub const TASKLESS_SOFTIRQ_ID: u8 = 2;
