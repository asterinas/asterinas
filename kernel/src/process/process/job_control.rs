// SPDX-License-Identifier: MPL-2.0

use ostd::sync::{LocalIrqDisabled, WaitQueue};

use super::{ProcessGroup, Session};
use crate::prelude::*;

/// The job control for terminals like TTY and PTY.
///
/// This structure is used to support the shell job control, allowing users to
/// run commands in the foreground or in the background. To achieve this, this
/// structure internally manages the session and the foreground process group
/// for a terminal.
pub struct JobControl {
    inner: SpinLock<Inner, LocalIrqDisabled>,
    wait_queue: WaitQueue,
}

#[derive(Default)]
struct Inner {
    session: Weak<Session>,
    foreground: Weak<ProcessGroup>,
}

impl JobControl {
    /// Creates a new `JobControl`.
    pub fn new() -> Self {
        Self {
            inner: SpinLock::new(Inner::default()),
            wait_queue: WaitQueue::new(),
        }
    }

    // *************** Session ***************

    /// Returns the session whose controlling terminal is this terminal.
    pub(super) fn session(&self) -> Option<Arc<Session>> {
        self.inner.lock().session.upgrade()
    }

    /// Sets the session whose controlling terminal is this terminal.
    ///
    /// # Errors
    ///
    /// This method will fail with `EPERM` if the terminal is already a controlling terminal of
    /// another session.
    ///
    /// # Panics
    ///
    /// The caller needs to ensure that the foreground process group actually belongs to the
    /// session. Otherwise, this method may panic.
    pub(super) fn set_session(
        &self,
        session: &Arc<Session>,
        foreground: &Arc<ProcessGroup>,
    ) -> Result<()> {
        let mut inner = self.inner.lock();

        if inner.session.upgrade().is_some() {
            return_errno_with_message!(
                Errno::EPERM,
                "the terminal is already a controlling terminal of another session"
            );
        }

        *inner = Inner {
            session: Arc::downgrade(session),
            foreground: Arc::downgrade(foreground),
        };

        Ok(())
    }

    /// Unsets the session because its controlling terminal is no longer this terminal.
    ///
    /// This method will return the foreground process group before the session is cleared.
    ///
    /// # Panics
    ///
    /// The caller needs to ensure that the session was previously set. Otherwise this method may
    /// panic.
    pub(super) fn unset_session(&self) -> Option<Arc<ProcessGroup>> {
        let mut inner = self.inner.lock();

        debug_assert!(inner.session.upgrade().is_some());

        let foreground = inner.foreground.upgrade();
        *inner = Inner::default();

        foreground
    }

    // *************** Foreground process group ***************

    /// Returns the foreground process group.
    pub fn foreground(&self) -> Option<Arc<ProcessGroup>> {
        self.inner.lock().foreground.upgrade()
    }

    /// Sets the foreground process group.
    ///
    /// # Panics
    ///
    /// The caller needs to ensure that foreground process group actually belongs to the session
    /// whose controlling terminal is this terminal. Otherwise this method may panic.
    pub(super) fn set_foreground(&self, process_group: &Arc<ProcessGroup>) {
        let mut inner = self.inner.lock();

        debug_assert!(Arc::ptr_eq(
            &process_group.session().unwrap(),
            &inner.session.upgrade().unwrap()
        ));
        inner.foreground = Arc::downgrade(process_group);

        self.wait_queue.wake_all();
    }

    /// Waits until the current process is in the foreground process group.
    ///
    /// Note that we only wait if the terminal is the our controlling terminal. If it isn't, the
    /// method returns immediately without an error. This should match the Linux behavior where the
    /// SIGTTIN won't be sent if we're reading the terminal that is the controlling terminal of
    /// another session.
    ///
    /// # Panics
    ///
    /// This method will panic if it is not called in the process context.
    pub fn wait_until_in_foreground(&self) -> Result<()> {
        let current = current!();

        self.wait_queue.pause_until(|| {
            let process_group_mut = current.process_group.lock();
            let process_group = process_group_mut.upgrade().unwrap();
            let session = process_group.session().unwrap();

            let inner = self.inner.lock();
            if !inner
                .session
                .upgrade()
                .is_some_and(|terminal_session| Arc::ptr_eq(&terminal_session, &session))
            {
                // The terminal is not our controlling terminal. Don't wait.
                return Some(());
            }

            inner
                .foreground
                .upgrade()
                .is_some_and(|terminal_foregroup| Arc::ptr_eq(&terminal_foregroup, &process_group))
                .then_some(())
        })
    }
}

impl Default for JobControl {
    fn default() -> Self {
        Self::new()
    }
}
