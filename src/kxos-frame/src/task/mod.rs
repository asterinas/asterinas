//! Tasks are the unit of code execution.

mod processor;
mod scheduler;
#[allow(clippy::module_inception)]
mod task;

pub use self::processor::get_idle_task_cx_ptr;
pub use self::scheduler::{set_scheduler, Scheduler};
pub use self::task::context_switch;
pub use self::task::TaskContext;
pub use self::task::SWITCH_TO_USER_SPACE_TASK;
pub use self::task::{Task, TaskStatus};
