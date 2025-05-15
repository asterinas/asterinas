// SPDX-License-Identifier: MPL-2.0

use super::{
    dealloc_unique_ids, get_init_pid_namespace, unique_ids_map::TaskListGuard, Process,
    ProcessGroup, UniqueIdsMap,
};
use crate::{
    prelude::*,
    process::pid_namespace::{UniqueIdsMapWithTasklistGuard, TASK_LIST_LOCK},
};

/// Collection of all [`UniqueIdsMap`] instances associated with a process.
///
/// This includes the [`UniqueIdsMap`] instances for the process itself,
/// the process group, and the session.
/// As the main thread shares the same [`UniqueIdsMap`] as the process,
/// we can utilize the process's [`UniqueIdsMap`] for the main thread.
///
/// The [`TaskListGuard`] is also included here,
/// enabling the manipulation of all [`UniqueIdsMap`] instances
/// without encountering race conditions.
pub struct MapsOfProcess {
    process: Option<UniqueIdsMap>,
    process_group: Option<UniqueIdsMap>,
    session: Option<UniqueIdsMap>,
    task_list_guard: Option<TaskListGuard>,
}

impl MapsOfProcess {
    /// Retrieves all [`UniqueIdsMap`] instances associated with a process
    /// and locks the task list.
    pub fn get_maps_and_lock_task_list(
        process: &Process,
        process_group_mut: &mut MutexGuard<'_, Weak<ProcessGroup>>,
    ) -> Self {
        let process_map = {
            let pid = process.unique_ids();
            process.pid_namespace().get_map_by_ids(pid)
        };

        let pgrp = process_group_mut.upgrade().unwrap();

        let pgrp_map = {
            let pgid = pgrp.unique_ids();
            // Note that the process group might not be visible in the process's PID namespace,
            // so we retrieve the process group from the init PID namespace.
            get_init_pid_namespace().get_map_by_ids(pgid)
        };

        let session_map = {
            let session = pgrp.session().unwrap();
            let sid = session.unique_ids();
            get_init_pid_namespace().get_map_by_ids(sid)
        };

        Self {
            process: process_map,
            process_group: pgrp_map,
            session: session_map,
            task_list_guard: Some(TASK_LIST_LOCK.lock()),
        }
    }
}

impl MapsOfProcess {
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

    pub fn process_map_guard(&mut self) -> UniqueIdsMapWithTasklistGuard<'_> {
        UniqueIdsMapWithTasklistGuard::new(
            self.process.as_ref().unwrap(),
            self.task_list_guard.as_mut().unwrap(),
        )
    }

    pub fn task_list_guard(&mut self) -> &mut TaskListGuard {
        self.task_list_guard.as_mut().unwrap()
    }
}

impl Drop for MapsOfProcess {
    fn drop(&mut self) {
        let process_ids = self.process.take().unwrap().ids().clone();
        let pgrp_ids = self.process_group.take().unwrap().ids().clone();
        let session_ids = self.session.take().unwrap().ids().clone();

        // The task list guard must be released before calling `dealloc_unique_ids` to prevent deadlock.
        // Note that the task list guard can not be passed to `dealloc_unique_ids`
        // to maintain consistent lock order. We should lock uid maps at frist, then task list.
        self.task_list_guard.take();

        dealloc_unique_ids(&process_ids);

        if pgrp_ids != process_ids {
            dealloc_unique_ids(&pgrp_ids);
        }

        if session_ids != process_ids && session_ids != pgrp_ids {
            dealloc_unique_ids(&session_ids);
        }
    }
}
