use crate::prelude::*;
use crate::user::UserSpace;

/// A task that executes a function to the end.
pub struct Task {
    func: Box<dyn FnOnce()>,
    data: Box<dyn Any + Send + Sync>,
    user_space: Option<UserSpace>,
}

impl Task {
    /// Gets the current task.
    pub fn current() -> Arc<Task> {
        todo!()
    }

    /// Yields execution so that another task may be scheduled.
    ///
    /// Note that this method cannot be simply named "yield" as the name is
    /// a Rust keyword.
    pub fn yield_now() {
        todo!()
    }

    /// Spawns a task that executes a function.
    ///
    /// Each task is associated with a per-task data and an optional user space.
    /// If having a user space, then the task can switch to the user space to
    /// execute user code. Multiple tasks can share a single user space.
    pub fn spawn<F, T>(
        task_fn: F,
        task_data: T,
        user_space: Option<Arc<UserSpace>>,
    ) -> Result<Arc<Self>>
    where
        F: FnOnce(),
        T: Any + Send + Sync,
    {
        todo!()
    }

    /// Returns the task status.
    pub fn status(&self) -> TaskStatus {
        todo!()
    }

    /// Returns the task data.
    pub fn data(&self) -> &dyn Any {
        todo!()
    }

    /// Returns the user space of this task, if it has.
    pub fn user_space(&self) -> Option<&Arc<UserSpace>> {
        todo!()
    }
}

/// The status of a task.
pub enum TaskStatus {
    /// The task is runnable.
    Runnable,
    /// The task is sleeping.
    Sleeping,
    /// The task has exited.
    Exited,
}
