//! Tasks are the unit of code execution.

mod nice;
mod preempt;
mod priority;
mod processor;
mod scheduler;
#[allow(clippy::module_inception)]
mod task;

pub use self::nice::Nice;
pub use self::preempt::{in_atomic, preemptible, DisablePreemptGuard};
pub use self::priority::Priority;
pub use self::processor::{
    current_task, init as init_processor, schedule, switch_to, try_preempt, yield_now,
};
pub use self::scheduler::{
    add_task, init as init_scheduler, remove_task, set_scheduler, Scheduler,
};
pub use self::task::{
    NeedResched, ReadPriority, Task, TaskAdapter, TaskOptions, TaskStatus, WakeUp,
};
