// SPDX-License-Identifier: MPL-2.0

//! Scheduler implementations.

// There may be multiple scheduling policies in the system,
// and subsequent schedulers can be placed under this module.
pub mod fifo_with_rt_preempt;
