// SPDX-License-Identifier: MPL-2.0

//! Task sets.

use ostd::task::{CurrentTask, Task};
use typeflags_util::bool;

use crate::{prelude::*, thread::AsThread};

/// A task set that maintains all tasks in a POSIX process.
pub struct TaskSet {
    tasks: Vec<Arc<Task>>,
    has_exited_main: bool,
    has_exited_group: bool,
    in_execve: bool,
}

impl TaskSet {
    /// Creates a new task set.
    pub(super) fn new() -> Self {
        Self {
            tasks: Vec::new(),
            has_exited_main: false,
            has_exited_group: false,
            in_execve: false,
        }
    }

    /// Inserts a new task to the task set.
    ///
    /// This method will fail if [`Self::set_exited_group`] or [`Self::set_in_execve`]
    /// has been called before.
    pub(super) fn insert(&mut self, task: Arc<Task>) -> core::result::Result<(), Arc<Task>> {
        if self.has_exited_group || self.in_execve {
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

    /// Sets the current task as the main task.
    ///
    /// # Panics
    ///
    /// The method will panics in following cases:
    /// 1. There are more threads other than the main thread and the current threads is
    ///    still in the taskset;
    /// 2. The main thread has not exited;
    /// 3. The current thread is not in the taskset.
    pub(super) fn set_main(&mut self, ctx: &Context) {
        assert_eq!(self.tasks.len(), 2);
        assert!(self.has_exited_main);
        assert!(self.tasks[0].as_thread().unwrap().is_exited());
        assert!(core::ptr::eq(ctx.task, self.tasks[1].as_ref()));

        self.tasks.swap_remove(0);
        self.has_exited_main = false;
    }

    /// Sets a flag that denotes that an `exit_group` has been initiated.
    pub(super) fn set_exited_group(&mut self) {
        debug_assert!(!self.in_execve);
        self.has_exited_group = true;
    }

    /// Returns whether an `exit_group` has been initiated.
    pub(super) fn has_exited_group(&self) -> bool {
        self.has_exited_group
    }

    /// Sets a flag that denoted whether an `execve` has been initiated and not finishes.
    pub(super) fn set_in_execve(&mut self) {
        self.in_execve = true;
    }

    /// Reset a flag to indicate an `execve` has finished.
    pub(super) fn reset_in_execve(&mut self) {
        self.in_execve = false;
    }

    /// Returns whether an `execve` has been initiated.
    pub(super) fn in_execve(&self) -> bool {
        self.in_execve
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
