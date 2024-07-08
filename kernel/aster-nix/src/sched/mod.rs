// SPDX-License-Identifier: MPL-2.0

mod priority;
mod priority_scheduler;

pub use priority::{AtomicPriority, Nice, Priority};
// There may be multiple scheduling policies in the system,
// and subsequent schedulers can be placed under this module.
pub use priority_scheduler::init;
