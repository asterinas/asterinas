// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use ostd::sync::WaitQueue;

use crate::{
    prelude::*,
    process::{
        signal::{
            constants::{SIGCONT, SIGHUP},
            signals::kernel::KernelSignal,
        },
        ProcessGroup, Session,
    },
};

/// The job control for terminals like tty and pty.
///
/// This struct is used to support shell job control, which allows users to
/// run commands in the foreground or in the background. This struct manages
/// the session and foreground process group for a terminal.
pub struct JobControl {
    foreground: SpinLock<Weak<ProcessGroup>>,
    session: SpinLock<Weak<Session>>,
    wait_queue: WaitQueue,
}

impl JobControl {
    /// Creates a new `TtyJobControl`
    pub fn new() -> Self {
        Self {
            foreground: SpinLock::new(Weak::new()),
            session: SpinLock::new(Weak::new()),
            wait_queue: WaitQueue::new(),
        }
    }

    // *************** Session ***************

    /// Returns the session whose controlling terminal is the terminal.
    fn session(&self) -> Option<Arc<Session>> {
        self.session.lock().upgrade()
    }

    /// Sets the terminal as the controlling terminal of the `session`.
    ///
    /// # Panics
    ///
    /// This terminal should not belong to any session.
    pub fn set_session(&self, session: &Arc<Session>) {
        debug_assert!(self.session().is_none());
        *self.session.lock() = Arc::downgrade(session);
    }

    /// Sets the terminal as the controlling terminal of the session of current process.
    ///
    /// # Panics
    ///
    /// This function should only be called in process context.
    pub fn set_current_session(&self) -> Result<()> {
        if self.session().is_some() {
            return_errno_with_message!(
                Errno::EPERM,
                "the terminal is already controlling terminal of another session"
            );
        }

        let current = current!();

        let process_group = current.process_group().unwrap();
        *self.foreground.lock() = Arc::downgrade(&process_group);

        let session = current.session().unwrap();
        *self.session.lock() = Arc::downgrade(&session);

        self.wait_queue.wake_all();
        Ok(())
    }

    /// Releases the current session from this terminal.
    pub fn release_current_session(&self) -> Result<()> {
        let Some(session) = self.session() else {
            return_errno_with_message!(
                Errno::ENOTTY,
                "the terminal is not controlling terminal now"
            );
        };

        if let Some(foreground) = self.foreground() {
            foreground.broadcast_signal(KernelSignal::new(SIGHUP));
            foreground.broadcast_signal(KernelSignal::new(SIGCONT));
        }

        Ok(())
    }

    // *************** Foreground process group ***************

    /// Returns the foreground process group
    pub fn foreground(&self) -> Option<Arc<ProcessGroup>> {
        self.foreground.lock().upgrade()
    }

    /// Sets the foreground process group.
    ///
    /// # Panics
    ///
    /// The process group should belong to one session.
    pub fn set_foreground(&self, process_group: Option<&Arc<ProcessGroup>>) -> Result<()> {
        let Some(process_group) = process_group else {
            // FIXME: should we allow this branch?
            *self.foreground.lock() = Weak::new();
            return Ok(());
        };

        let session = process_group.session().unwrap();
        let Some(terminal_session) = self.session() else {
            return_errno_with_message!(
                Errno::EPERM,
                "the terminal does not become controlling terminal of one session."
            );
        };

        if !Arc::ptr_eq(&terminal_session, &session) {
            return_errno_with_message!(
                Errno::EPERM,
                "the process proup belongs to different session"
            );
        }

        *self.foreground.lock() = Arc::downgrade(process_group);
        self.wait_queue.wake_all();
        Ok(())
    }

    /// Wait until the current process is the foreground process group. If
    /// the foreground process group is None, returns true.
    ///
    /// # Panics
    ///
    /// This function should only be called in process context.
    pub fn wait_until_in_foreground(&self) -> Result<()> {
        // Fast path
        if self.current_belongs_to_foreground() {
            return Ok(());
        }

        // Slow path
        self.wait_queue.pause_until(|| {
            if self.current_belongs_to_foreground() {
                Some(())
            } else {
                None
            }
        })
    }

    fn current_belongs_to_foreground(&self) -> bool {
        let Some(foreground) = self.foreground() else {
            return true;
        };

        foreground.contains_process(current!().pid())
    }
}

impl Default for JobControl {
    fn default() -> Self {
        Self::new()
    }
}
