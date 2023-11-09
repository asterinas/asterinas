//! Tasks are the unit of code execution.

mod preempt;
mod priority;
mod processor;
mod scheduler;
#[allow(clippy::module_inception)]
mod task;

use self::preempt::preempt_stat;
pub use self::preempt::{in_atomic, in_irq, preemptible, DisablePreemptGuard};
pub use self::priority::Priority;
pub(crate) use self::processor::scheduler_tick;
pub use self::processor::{current_task, preempt, schedule};
pub use self::scheduler::{add_task, set_scheduler, Scheduler};
pub use self::task::{Task, TaskAdapter, TaskOptions, TaskStatus};
