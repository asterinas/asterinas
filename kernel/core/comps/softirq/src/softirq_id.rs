// SPDX-License-Identifier: MPL-2.0

//! Defines the used IDs of software interrupt (softirq) lines.

/// The corresponding softirq line is used to schedule urgent taskless jobs.
pub const TASKLESS_URGENT_SOFTIRQ_ID: u8 = 0;

/// The corresponding softirq line is used to manage timers and handle
/// time-related jobs.
pub const TIMER_SOFTIRQ_ID: u8 = 1;

/// The corresponding softirq line is used to schedule general taskless jobs.
pub const TASKLESS_SOFTIRQ_ID: u8 = 2;

/// The corresponding softirq line is used to handle transmission network events.
pub const NETWORK_TX_SOFTIRQ_ID: u8 = 3;

/// The corresponding softirq line is used to handle reception network events.
pub const NETWORK_RX_SOFTIRQ_ID: u8 = 4;
