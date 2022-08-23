//! Tasks are the unit of code execution.

mod processor;
mod scheduler;
#[allow(clippy::module_inception)]
mod task;

pub use self::scheduler::{set_scheduler, Scheduler};
pub use self::task::{Task, TaskStatus};
