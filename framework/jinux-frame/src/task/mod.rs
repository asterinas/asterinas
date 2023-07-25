//! Tasks are the unit of code execution.

mod processor;
mod scheduler;
#[allow(clippy::module_inception)]
mod task;

pub use self::processor::{current_task, disable_preempt, schedule, DisablePreemptGuard};
pub use self::scheduler::{add_task, set_scheduler, Scheduler};
pub use self::task::{Task, TaskAdapter, TaskStatus};
