// SPDX-License-Identifier: MPL-2.0

use core::ops::Deref;

use super::{Pgid, Pid, Process, Sid};
use crate::{
    prelude::*,
    process::{pid_namespace::MapsOfProcess, posix_thread::CurrentPosixThread},
};

/// The current process.
pub struct CurrentProcess(Arc<Process>);

impl CurrentProcess {
    /// Returns the process's ID.
    pub fn pid(&self) -> Pid {
        self.0.pid
    }

    /// Returns the ID of the parent process.
    pub fn parent_pid(&self) -> Pid {
        self.parent.pid()
    }

    /// Returns the process group ID of the process.
    pub fn pgid(&self) -> Pgid {
        let Some(pgrp) = self.process_group.lock().upgrade() else {
            return 0;
        };

        pgrp.pgid_in_ns(&self.pid_namespace).unwrap_or(0)
    }

    /// Returns the session ID of the process.
    pub fn sid(&self) -> Sid {
        let Some(session) = self
            .process_group
            .lock()
            .upgrade()
            .and_then(|pgrp| pgrp.session())
        else {
            return 0;
        };

        session.sid_in_ns(&self.pid_namespace).unwrap_or(0)
    }

    /// Moves the process itself or its child process to another process group.
    ///
    /// The process to be moved is specified with the process ID `pid`; `self` is used only for
    /// permission checking purposes (see the Errors section below).
    ///
    /// If `pgid` is equal to the process ID, a new process group with the given PGID will be
    /// created (if it does not already exist). Then, the process will be moved to the process
    /// group with the given PGID, if the process group exists and belongs to the same session as
    /// the given process.
    ///
    /// # Errors
    ///
    /// This method will return `ESRCH` in following cases:
    ///  * The process specified by `pid` does not exist;
    ///  * The process specified by `pid` is neither `self` or a child process of `self`.
    ///
    /// This method will return `EPERM` in following cases:
    ///  * The process is not in the same session as `self`;
    ///  * The process is a session leader, but the given PGID is not the process's PID/PGID;
    ///  * The process group already exists, but the group does not belong to the same session;
    ///  * The process group does not exist, but `pgid` is not equal to the process ID.
    pub fn move_process_to_group(&self, pid: Pid, pgid: Pgid) -> Result<()> {
        let process_is_current = pid == self.pid;

        let process = if process_is_current {
            self.0.clone()
        } else {
            self.pid_namespace()
                .get_process(pid)
                .ok_or(Error::with_message(
                    Errno::ESRCH,
                    "the process to set the PGID does not exist",
                ))?
        };

        // Lock order: group of process -> task list
        // -> group inner -> session inner

        // We lock the process group of process with smaller `unique_ids` first.
        let (mut process_group_mut, current_process_group_mut) = if process_is_current {
            (process.process_group.lock(), None)
        } else if process.unique_ids < self.unique_ids {
            (
                process.process_group.lock(),
                Some(self.process_group.lock()),
            )
        } else {
            let current_process_group_mut = self.process_group.lock();
            (
                process.process_group.lock(),
                Some(current_process_group_mut),
            )
        };

        let mut maps_of_process =
            MapsOfProcess::get_maps_and_lock_task_list(&process, &mut process_group_mut);

        // After holding the task list lock, we need to do another check to ensure the process does exist.
        if maps_of_process
            .process_map_guard()
            .attached_process()
            .is_none()
        {
            return_errno_with_message!(Errno::ESRCH, "the process to set the PGID does not exist");
        }

        let current_session = if process_is_current {
            // There is no need to check if the session is the same in this case.
            None
        } else if let Some(ppid) = process.parent_pid_in_ns(&self.pid_namespace)
            && ppid == self.pid
        {
            // FIXME: If the child process has called `execve`, we should fail with `EACCESS`.
            Some(
                current_process_group_mut
                    .as_ref()
                    .unwrap()
                    .upgrade()
                    .unwrap()
                    .session()
                    .unwrap(),
            )
        } else {
            return_errno_with_message!(
                Errno::ESRCH,
                "the process to set the PGID is neither the current process nor its child process"
            );
        };

        drop(current_process_group_mut);

        let process_group = self
            .pid_namespace
            .get_map_by_id(pgid)
            .map(|unique_ids_map| {
                unique_ids_map
                    .with_task_list_guard(maps_of_process.task_list_guard())
                    .attached_process_group()
            })
            .flatten();

        if let Some(process_group) = process_group {
            process.to_existing_group(
                current_session,
                &mut process_group_mut,
                &mut maps_of_process,
                process_group,
            )
        } else if pgid == process.pid {
            process.to_new_group(
                current_session,
                &mut process_group_mut,
                &mut maps_of_process,
            )
        } else {
            return_errno_with_message!(Errno::EPERM, "the new process group does not exist");
        }
    }

    /// Moves the process to the new session.
    ///
    /// This method will create a new process group in a new session, move the process to the new
    /// session, and return the session ID (which is equal to the process ID and the process group
    /// ID).
    ///
    /// # Errors
    ///
    /// This method will return `EPERM` if an existing process group has the same identifier as the
    /// process ID. This means that the process is or was a process group leader and that the
    /// process group is still alive.
    pub fn to_new_session(&self) -> Result<Sid> {
        // Lock order: group of process -> task list -> group inner -> session inner
        let mut process_group_mut = self.process_group.lock();
        let mut maps_of_process =
            MapsOfProcess::get_maps_and_lock_task_list(self, &mut process_group_mut);

        if maps_of_process
            .process_map_guard()
            .attached_session()
            .is_some()
        {
            // FIXME: According to the Linux implementation, this check should be removed, so we'll
            // return `EPERM` due to hitting the following check. However, we need to work around a
            // gVisor bug. The upstream gVisor has fixed the issue in:
            // <https://github.com/google/gvisor/commit/582f7bf6c0ccccaeb1215a232709df38d5d409f7>.
            return Ok(self.pid);
        }
        if maps_of_process
            .process_map_guard()
            .attached_process_group()
            .is_some()
        {
            return_errno_with_message!(
                Errno::EPERM,
                "a process group leader cannot be moved to a new session"
            );
        }

        self.clear_old_group_and_session(&mut process_group_mut, &mut maps_of_process);

        Ok(self.set_new_session(
            &mut process_group_mut,
            &mut maps_of_process.process_map_guard(),
        ))
    }
}

impl !Send for CurrentProcess {}
impl !Sync for CurrentProcess {}

impl Deref for CurrentProcess {
    type Target = Arc<Process>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub trait AsCurrentProcess {
    /// Returns the associated process if `self` is the current thread.
    fn as_current_process(&self) -> CurrentProcess;
}

impl AsCurrentProcess for CurrentPosixThread<'_> {
    fn as_current_process(&self) -> CurrentProcess {
        CurrentProcess(self.deref().process())
    }
}
