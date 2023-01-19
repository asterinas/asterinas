//! Tasks are the unit of code execution.

mod processor;
mod scheduler;
#[allow(clippy::module_inception)]
mod task;

pub(crate) use self::processor::get_idle_task_cx_ptr;
pub use self::processor::schedule;
pub use self::scheduler::{set_scheduler, Scheduler};
pub(crate) use self::task::context_switch;
pub(crate) use self::task::TaskContext;
pub(crate) use self::task::SWITCH_TO_USER_SPACE_TASK;
pub use self::task::{Task, TaskStatus};
