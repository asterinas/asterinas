// SPDX-License-Identifier: MPL-2.0

use super::{
    dealloc_ns_pids, get_init_pid_namespace, pid_entry::TaskListGuard, PidEntry, Process,
    ProcessGroup,
};
use crate::{prelude::*, process::pid_namespace::PidEntryWithTasklistGuard};

/// Collection of all [`PidEntry`] instances associated with a process.
///
/// This includes the [`PidEntry`] instances for the process itself,
/// the process group, and the session.
/// As the main thread shares the same [`PidEntry`] as the process,
/// we can utilize the process's [`PidEntry`] for the main thread.
///
/// The [`TaskListGuard`] is also included here,
/// enabling the manipulation of all [`PidEntry`] instances
/// without encountering race conditions.
pub struct ProcessPidEntries {
    process: Option<PidEntry>,
    process_group: Option<PidEntry>,
    session: Option<PidEntry>,
    task_list_guard: Option<TaskListGuard>,
}

impl ProcessPidEntries {
    /// Retrieves all [`PidEntry`] instances associated with a process.
    pub fn get_entries(
        process: &Process,
        task_list_guard: TaskListGuard,
        process_group_mut: &mut Weak<ProcessGroup>,
    ) -> Self {
        let process_entry = {
            let pid = process.ns_pids();
            process.pid_namespace().get_entry_by_ids(pid)
        };

        let pgrp = process_group_mut.upgrade().unwrap();

        let pgrp_entry = {
            let pgid = pgrp.ns_pgids();
            // Note that the process group might not be visible in the process's PID namespace,
            // so we retrieve the process group from the init PID namespace.
            get_init_pid_namespace().get_entry_by_ids(pgid)
        };

        let session_entry = {
            let session = pgrp.session().unwrap();
            let sid = session.ns_sids();
            get_init_pid_namespace().get_entry_by_ids(sid)
        };

        Self {
            process: process_entry,
            process_group: pgrp_entry,
            session: session_entry,
            task_list_guard: Some(task_list_guard),
        }
    }
}

impl ProcessPidEntries {
    pub fn detach_thread(&mut self) {
        self.process
            .as_ref()
            .unwrap()
            .with_task_list_guard(self.task_list_guard.as_mut().unwrap())
            .detach_thread();
    }

    pub fn detach_process(&mut self) {
        self.process
            .as_ref()
            .unwrap()
            .with_task_list_guard(self.task_list_guard.as_mut().unwrap())
            .detach_process();
    }

    pub fn detach_process_group(&mut self) {
        self.process_group
            .as_ref()
            .unwrap()
            .with_task_list_guard(self.task_list_guard.as_mut().unwrap())
            .detach_process_group();
    }

    pub fn detach_session(&mut self) {
        self.session
            .as_ref()
            .unwrap()
            .with_task_list_guard(self.task_list_guard.as_mut().unwrap())
            .detach_session();
    }

    pub fn process_entry_guard(&mut self) -> PidEntryWithTasklistGuard<'_> {
        PidEntryWithTasklistGuard::new(
            self.process.as_ref().unwrap(),
            self.task_list_guard.as_mut().unwrap(),
        )
    }

    pub fn task_list_guard(&mut self) -> &mut TaskListGuard {
        self.task_list_guard.as_mut().unwrap()
    }
}

impl Drop for ProcessPidEntries {
    fn drop(&mut self) {
        let process_ids = self.process.take().unwrap().ids().clone();
        let pgrp_ids = self.process_group.take().unwrap().ids().clone();
        let session_ids = self.session.take().unwrap().ids().clone();

        // The task list guard must be released before calling `dealloc_ns_pids` to prevent deadlock.
        // Note that the task list guard can not be passed to `dealloc_ns_pids`
        // to maintain consistent lock order.
        // In `dealloc_ns_pids`, we will lock PID_ENTRY_MAP_LOCK at first, then task list.
        self.task_list_guard.take();

        dealloc_ns_pids(&process_ids);

        if pgrp_ids != process_ids {
            dealloc_ns_pids(&pgrp_ids);
        }

        if session_ids != process_ids && session_ids != pgrp_ids {
            dealloc_ns_pids(&session_ids);
        }
    }
}
