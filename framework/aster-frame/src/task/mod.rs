// SPDX-License-Identifier: MPL-2.0

//! Tasks are the unit of code execution.

mod priority;
mod processor;
mod scheduler;
#[allow(clippy::module_inception)]
mod task;

pub use self::priority::Priority;
pub use self::processor::{current_task, disable_preempt, preempt, schedule, DisablePreemptGuard};
pub use self::scheduler::{add_task, set_scheduler, Scheduler};
pub use self::task::{Task, TaskAdapter, TaskOptions, TaskStatus};
