// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use super::{session::SessionGuard, JobControl, Pgid, Process, Session};
use crate::{
    fs::device::Device,
    prelude::{current, return_errno_with_message, warn, Errno, Error, Result},
    process::process_table,
    util::ioctl::{dispatch_ioctl, RawIoctl},
};

/// A terminal.
///
/// We currently support two kinds of terminal, the TTY and pty. They're associated with a
/// `JobControl` to track the session and the foreground process group.
pub trait Terminal: Device {
    /// Returns the job control of the terminal.
    fn job_control(&self) -> &JobControl;
}

mod ioctl_defs {
    use crate::{
        process::{Pgid, Sid},
        util::ioctl::{ioc, InData, NoData, OutData, PassByVal},
    };

    // Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/asm-generic/ioctls.h>

    pub(super) type GetForegroundPgid = ioc!(TIOCGPGRP, 0x540F, OutData<Pgid>);
    pub(super) type SetForegroundPgid = ioc!(TIOCSPGRP, 0x5410, InData<Pgid>);

    pub(super) type SetControlTty     = ioc!(TIOCSCTTY, 0x540E, InData<i32, PassByVal>);
    pub(super) type SetControlNoTty   = ioc!(TIOCNOTTY, 0x5422, NoData);
    pub(super) type GetControlSid     = ioc!(TIOCGSID,  0x5429, OutData<Sid>);
}

impl dyn Terminal {
    pub fn job_ioctl(self: Arc<Self>, raw_ioctl: RawIoctl, via_master: bool) -> Result<()> {
        use ioctl_defs::*;

        dispatch_ioctl!(match raw_ioctl {
            // Commands about foreground process groups
            cmd @ GetForegroundPgid => {
                let operate = || {
                    self.job_control()
                        .foreground()
                        .map_or(0, |foreground| foreground.pgid())
                };

                let pgid = if via_master {
                    operate()
                } else {
                    self.is_control_and(&current!(), |_, _| Ok(operate()))?
                };

                cmd.write(&pgid)
            }
            cmd @ SetForegroundPgid => {
                let pgid = cmd.read()?;
                if pgid.cast_signed() < 0 {
                    return_errno_with_message!(Errno::EINVAL, "negative PGIDs are not valid");
                }

                self.set_foreground(pgid, &current!())
            }

            // Commands about sessions
            cmd @ SetControlTty => {
                if cmd.get() == 1 {
                    warn!("stealing TTY from another session is not supported");
                }

                self.set_control(&current!())
            }
            _cmd @ SetControlNoTty => {
                if via_master {
                    return_errno_with_message!(
                        Errno::ENOTTY,
                        "the terminal to operate is not our controlling terminal"
                    );
                }

                self.unset_control(&current!())
            }
            cmd @ GetControlSid => {
                let sid = if via_master {
                    self.job_control()
                        .session()
                        .ok_or_else(|| {
                            Error::with_message(
                                Errno::ENOTTY,
                                "the terminal is not a controlling termainal of any session",
                            )
                        })?
                        .sid()
                } else {
                    self.is_control_and(&current!(), |session, _| Ok(session.sid()))?
                };

                cmd.write(&sid)
            }

            // Commands that are invalid or not supported
            _ => {
                return_errno_with_message!(Errno::ENOTTY, "the ioctl command is unknown")
            }
        })
    }

    /// Sets the terminal to be the controlling terminal of the process.
    pub(super) fn set_control(self: Arc<Self>, process: &Process) -> Result<()> {
        // Lock order: group of process -> session inner -> job control
        let process_group_mut = process.process_group.lock();

        let process_group = process_group_mut.upgrade().unwrap();
        let session = process_group.session().unwrap();

        if !session.is_leader(process) {
            return_errno_with_message!(
                Errno::EPERM,
                "the process who sets the controlling terminal is not a session leader"
            );
        };

        let mut session_inner = session.lock();

        if let Some(session_terminal) = session_inner.terminal() {
            if Arc::ptr_eq(session_terminal, &self) {
                return Ok(());
            }
            return_errno_with_message!(
                Errno::EPERM,
                "the session already has a controlling terminal"
            );
        }

        self.job_control().set_session(&session, &process_group)?;
        session_inner.set_terminal(Some(self));

        Ok(())
    }

    /// Unsets the terminal from the controlling terminal of the process.
    fn unset_control(self: Arc<Self>, process: &Process) -> Result<()> {
        // Lock order: group of process -> session inner -> job control
        self.is_control_and(process, |session, session_inner| {
            if !session.is_leader(process) {
                // TODO: The Linux kernel keeps track of the controlling terminal of each process
                // in `current->signal->tty`. So even if we're not the session leader, this may
                // still succeed in releasing the controlling terminal of the current process. Note
                // that the controlling terminal of the session will never be released in this
                // case. We cannot mimic the exact Linux behavior, so we just return `Ok(())` here.
                return Ok(());
            }

            session_inner.set_terminal(None);
            if let Some(foreground) = self.job_control().unset_session() {
                use crate::process::signal::{
                    constants::{SIGCONT, SIGHUP},
                    signals::kernel::KernelSignal,
                };
                // FIXME: Correct the lock order here. We cannot lock the group inner after locking
                // the session inner.
                foreground.broadcast_signal(KernelSignal::new(SIGHUP));
                foreground.broadcast_signal(KernelSignal::new(SIGCONT));
            }

            Ok(())
        })
    }

    /// Sets the foreground process group of the terminal.
    fn set_foreground(self: Arc<Self>, pgid: Pgid, process: &Process) -> Result<()> {
        // Lock order: group table -> group of process -> session inner -> job control
        let group_table_mut = process_table::group_table_mut();

        self.is_control_and(process, |session, _| {
            let Some(process_group) = group_table_mut.get(&pgid) else {
                return_errno_with_message!(
                    Errno::ESRCH,
                    "the process group to be foreground does not exist"
                );
            };

            if !Arc::ptr_eq(session, &process_group.session().unwrap()) {
                return_errno_with_message!(
                    Errno::EPERM,
                    "the process group to be foreground belongs to a different session"
                );
            }

            self.job_control().set_foreground(process_group);

            Ok(())
        })
    }

    /// Runs `op` when the process controls the terminal.
    ///
    /// Note that this requires that the terminal is the controlling terminal of the session, but
    /// does _not_ require that the process is a session leader.
    fn is_control_and<F, R>(self: &Arc<Self>, process: &Process, op: F) -> Result<R>
    where
        F: FnOnce(&Arc<Session>, &mut SessionGuard) -> Result<R>,
    {
        let process_group_mut = process.process_group.lock();

        let process_group = process_group_mut.upgrade().unwrap();
        let session = process_group.session().unwrap();

        let mut session_inner = session.lock();

        if !session_inner
            .terminal()
            .is_some_and(|session_terminal| Arc::ptr_eq(session_terminal, self))
        {
            return_errno_with_message!(
                Errno::ENOTTY,
                "the terminal to operate is not our controlling terminal"
            );
        }

        op(&session, &mut session_inner)
    }
}
