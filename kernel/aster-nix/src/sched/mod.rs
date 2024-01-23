// SPDX-License-Identifier: MPL-2.0

pub mod nice;
mod priority_scheduler;

// There may be multiple scheduling policies in the system,
// and subsequent schedulers can be placed under this module.
pub use self::priority_scheduler::init;
