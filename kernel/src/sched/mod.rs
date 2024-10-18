// SPDX-License-Identifier: MPL-2.0

pub mod priority;
mod priority_scheduler;
mod sched_class;

// There may be multiple scheduling policies in the system,
// and subsequent schedulers can be placed under this module.
pub use self::sched_class::{init, SchedEntity};
