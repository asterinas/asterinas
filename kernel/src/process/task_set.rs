// SPDX-License-Identifier: MPL-2.0

//! Task sets.

use ostd::task::{CurrentTask, Task};

use crate::prelude::*;

/// A task set that maintains all tasks in a POSIX process.
pub struct TaskSet {
    tasks: Vec<Arc<Task>>,
    has_exited_main: bool,
    has_exited_group: bool,
}

impl TaskSet {
    /// Creates a new task set.
    pub(super) fn new() -> Self {
        Self {
            tasks: Vec::new(),
            has_exited_main: false,
            has_exited_group: false,
        }
    }

    /// Inserts a new task to the task set.
    ///
    /// This method will fail if [`Self::set_exited_group`] has been called before.
    pub(super) fn insert(&mut self, task: Arc<Task>) -> core::result::Result<(), Arc<Task>> {
        if self.has_exited_group {
            return Err(task);
        }

        self.tasks.push(task);
        Ok(())
    }

    /// Removes the exited task from the task set if necessary.
    ///
    /// The task will be removed from the task set if the corresponding thread is not the main
    /// thread.
    ///
    /// This method will return true if there are no more alive tasks in the task set.
    ///
    /// # Panics
    ///
    /// This method will panic if the task is not in the task set.
    pub(super) fn remove_exited(&mut self, task: &CurrentTask) -> bool {
        let position = self
            .tasks
            .iter()
            .position(|some_task| core::ptr::eq(some_task.as_ref(), task.as_ref()))
            .unwrap();

        if position == 0 {
            assert!(!self.has_exited_main);
            self.has_exited_main = true;
        } else {
            self.tasks.swap_remove(position);
        }

        self.has_exited_main && self.tasks.len() == 1
    }

    /// Sets a flag that denotes that an `exit_group` has been initiated.
    pub(super) fn set_exited_group(&mut self) {
        self.has_exited_group = true;
    }

    /// Returns whether an `exit_group` has been initiated.
    pub(super) fn has_exited_group(&self) -> bool {
        self.has_exited_group
    }
}

impl TaskSet {
    /// Returns a slice of the tasks in the task set.
    pub fn as_slice(&self) -> &[Arc<Task>] {
        self.tasks.as_slice()
    }

    /// Returns the main task/thread.
    pub fn main(&self) -> &Arc<Task> {
        &self.tasks[0]
    }
}
