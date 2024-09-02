// SPDX-License-Identifier: MPL-2.0

use super::JobControl;
use crate::{
    fs::inode_handle::FileIo,
    prelude::*,
    process::{process_table, Pgid, ProcessGroup},
};

/// A terminal is used to interact with system. A terminal can support the shell
/// job control.
///
/// We currently support two kinds of terminal, the tty and pty.
pub trait Terminal: FileIo {
    // *************** Foreground ***************

    /// Returns the foreground process group
    fn foreground(&self) -> Option<Arc<ProcessGroup>> {
        self.job_control().foreground()
    }

    /// Sets the foreground process group of this terminal.
    ///
    /// If the terminal is not controlling terminal, this method returns `ENOTTY`.
    ///
    /// # Panics
    ///
    /// This method should be called in process context.
    fn set_foreground(&self, pgid: &Pgid) -> Result<()> {
        if !self.is_controlling_terminal() {
            return_errno_with_message!(Errno::ENOTTY, "self is not controlling terminal");
        }

        let foreground = process_table::get_process_group(pgid);

        self.job_control().set_foreground(foreground.as_ref())
    }

    // *************** Session and controlling terminal ***************

    /// Returns whether the terminal is the controlling terminal of current process.
    ///
    /// # Panics
    ///
    /// This method should be called in process context.
    fn is_controlling_terminal(&self) -> bool {
        let session = current!().session().unwrap();
        let Some(terminal) = session.terminal() else {
            return false;
        };

        let arc_self = self.arc_self();
        Arc::ptr_eq(&terminal, &arc_self)
    }

    /// Sets the terminal as the controlling terminal of the session of current process.
    ///
    /// If self is not session leader, or the terminal is controlling terminal of other session,
    /// or the session already has controlling terminal, this method returns `EPERM`.
    ///
    /// # Panics
    ///
    /// This method should only be called in process context.
    fn set_current_session(&self) -> Result<()> {
        if !current!().is_session_leader() {
            return_errno_with_message!(Errno::EPERM, "current process is not session leader");
        }

        let get_terminal = || {
            self.job_control().set_current_session()?;
            Ok(self.arc_self())
        };

        let session = current!().session().unwrap();
        session.set_terminal(get_terminal)
    }

    /// Releases the terminal from the session of current process if the terminal is the controlling
    /// terminal of the session.
    ///
    /// If the terminal is not the controlling terminal of the session, this method will return `ENOTTY`.
    ///
    /// # Panics
    ///
    /// This method should only be called in process context.
    fn release_current_session(&self) -> Result<()> {
        if !self.is_controlling_terminal() {
            return_errno_with_message!(Errno::ENOTTY, "release wrong tty");
        }

        let current = current!();
        if !current.is_session_leader() {
            warn!("TODO: release tty for process that is not session leader");
            return Ok(());
        }

        let release_session = |_: &Arc<dyn Terminal>| self.job_control().release_current_session();

        let session = current.session().unwrap();
        session.release_terminal(release_session)
    }

    /// Returns the job control of the terminal.
    fn job_control(&self) -> &JobControl;

    fn arc_self(&self) -> Arc<dyn Terminal>;
}
