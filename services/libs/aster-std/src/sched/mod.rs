// SPDX-License-Identifier: MPL-2.0

mod priority_scheduler;

// There may be multiple scheduling policies in the system,
// and subsequent schedulers can be placed under this module.
pub use self::priority_scheduler::init;
