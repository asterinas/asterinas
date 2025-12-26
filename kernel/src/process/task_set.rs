// SPDX-License-Identifier: MPL-2.0

//! Task sets.

use ostd::{
    sync::Waker,
    task::{CurrentTask, Task},
};

use super::Pid;
use crate::{
    events::{Events, Observer, Subject},
    prelude::*,
    thread::Tid,
};

/// A task set that maintains all tasks in a POSIX process.
pub struct TaskSet {
    tasks: Vec<Arc<Task>>,
    has_exited_main: bool,
    has_exited_group: bool,
    in_execve: bool,
    execve_waker: Option<Arc<Waker>>,
    subject: Subject<TidEvent>,
}

impl TaskSet {
    /// Creates a new task set.
    pub(super) fn new() -> Self {
        Self {
            tasks: Vec::new(),
            has_exited_main: false,
            has_exited_group: false,
            in_execve: false,
            execve_waker: None,
            subject: Subject::new(),
        }
    }

    /// Inserts a new task to the task set.
    ///
    /// This method will fail if [`Self::set_exited_group`] or [`Self::start_execve`]
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
    pub(super) fn remove_exited(&mut self, task: &CurrentTask, tid: Tid) -> bool {
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
            self.notify_tid_exit(tid);
        }

        if let Some(waker) = self.execve_waker.as_ref() {
            waker.wake_up();
        }

        self.has_exited_main && self.tasks.len() == 1
    }

    /// Return whether the main thread has exited.
    pub(super) fn has_exited_main(&self) -> bool {
        self.has_exited_main
    }

    /// Removes the main task and makes the remaining task become the main task.
    ///
    /// The method should only be calling when doing execve.
    pub(super) fn swap_main(&mut self, pid: Pid, tid: Tid) {
        // This is an extremely internal method. The caller must uphold certain invariants, update
        // the thread status, modify the thread table, etc.

        self.tasks.swap_remove(0);
        self.has_exited_main = false;

        self.notify_tid_exit(pid);
        self.notify_tid_exit(tid);
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

    /// Sets a flag that denotes that an `execve` has been initiated.
    pub(super) fn start_execve(&mut self) {
        debug_assert!(!self.has_exited_group);
        self.in_execve = true;
    }

    /// Resets a flag to indicate an `execve` has finished.
    pub(super) fn finish_execve(&mut self) {
        self.in_execve = false;
    }

    /// Returns whether an `execve` has been initiated.
    pub(super) fn in_execve(&self) -> bool {
        self.in_execve
    }

    /// Registers a waker to be notified when any thread exits.
    ///
    /// Only a thread performing execve should set this waker; it is used to
    /// wake the execve-ing thread while it waits for other threads to exit.
    pub(super) fn set_execve_waker(&mut self, waker: Arc<Waker>) {
        debug_assert!(self.execve_waker.is_none());
        self.execve_waker = Some(waker);
    }

    /// Clears the waker previously set by [`Self::set_execve_waker`].
    pub(super) fn clear_execve_waker(&mut self) {
        self.execve_waker = None;
    }

    /// Notifies `TidEvent::Exit` events to the subject.
    fn notify_tid_exit(&mut self, tid: Tid) {
        self.subject.notify_observers(&TidEvent::Exit(tid));
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

    /// Registers an observer which watches `TidEvent`.
    pub fn register_observer(&mut self, observer: Weak<dyn Observer<TidEvent>>) {
        self.subject.register_observer(observer);
    }

    /// Unregisters an observer which watches `TidEvent`.
    #[expect(dead_code)]
    pub fn unregister_observer(&mut self, observer: &Weak<dyn Observer<TidEvent>>) {
        self.subject.unregister_observer(observer);
    }
}

#[derive(Copy, Clone)]
pub enum TidEvent {
    Exit(Tid),
}

impl Events for TidEvent {}
