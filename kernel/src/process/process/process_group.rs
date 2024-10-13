// SPDX-License-Identifier: MPL-2.0

use alloc::collections::btree_map::Values;

use super::{Pgid, Pid, Process, Session};
use crate::{prelude::*, process::signal::signals::Signal};

/// `ProcessGroup` represents a set of processes. Each `ProcessGroup` has a unique
/// identifier `pgid`.
pub struct ProcessGroup {
    pgid: Pgid,
    pub(in crate::process) inner: Mutex<Inner>,
}

pub(in crate::process) struct Inner {
    pub(in crate::process) processes: BTreeMap<Pid, Arc<Process>>,
    pub(in crate::process) leader: Option<Arc<Process>>,
    pub(in crate::process) session: Weak<Session>,
}

impl Inner {
    pub(in crate::process) fn remove_process(&mut self, pid: &Pid) {
        let Some(process) = self.processes.remove(pid) else {
            return;
        };

        if let Some(leader) = &self.leader
            && Arc::ptr_eq(leader, &process)
        {
            self.leader = None;
        }
    }

    pub(in crate::process) fn is_empty(&self) -> bool {
        self.processes.is_empty()
    }
}

impl ProcessGroup {
    /// Creates a new process group with one process. The pgid is the same as the process
    /// id. The process will become the leading process of the new process group.
    ///
    /// The caller needs to ensure that the process does not belong to any group.
    pub(in crate::process) fn new(process: Arc<Process>) -> Arc<Self> {
        let pid = process.pid();

        let inner = {
            let mut processes = BTreeMap::new();
            processes.insert(pid, process.clone());
            Inner {
                processes,
                leader: Some(process.clone()),
                session: Weak::new(),
            }
        };

        Arc::new(ProcessGroup {
            pgid: pid,
            inner: Mutex::new(inner),
        })
    }

    /// Returns whether self contains a process with `pid`.
    pub(in crate::process) fn contains_process(&self, pid: Pid) -> bool {
        self.inner.lock().processes.contains_key(&pid)
    }

    /// Returns the process group identifier
    pub fn pgid(&self) -> Pgid {
        self.pgid
    }

    /// Acquires a lock on the process group.
    pub fn lock(&self) -> ProcessGroupGuard {
        ProcessGroupGuard {
            inner: self.inner.lock(),
        }
    }

    /// Broadcasts signal to all processes in the group.
    ///
    /// This method should only be used to broadcast fault signal and kernel signal.
    ///
    /// TODO: do more check to forbid user signal
    pub fn broadcast_signal(&self, signal: impl Signal + Clone + 'static) {
        for process in self.inner.lock().processes.values() {
            process.enqueue_signal(signal.clone());
        }
    }

    /// Returns the leader process.
    pub fn leader(&self) -> Option<Arc<Process>> {
        self.inner.lock().leader.clone()
    }

    /// Returns the session which the group belongs to
    pub fn session(&self) -> Option<Arc<Session>> {
        self.inner.lock().session.upgrade()
    }
}

/// A scoped lock for a process group.
///
/// It provides some public methods to prevent the exposure of the inner type.
#[clippy::has_significant_drop]
#[must_use]
pub struct ProcessGroupGuard<'a> {
    inner: MutexGuard<'a, Inner>,
}

impl ProcessGroupGuard<'_> {
    /// Returns an iterator over the processes in the group.
    pub fn iter(&self) -> ProcessGroupIter {
        ProcessGroupIter {
            inner: self.inner.processes.values(),
        }
    }
}

/// An iterator over the processes of the process group.
pub struct ProcessGroupIter<'a> {
    inner: Values<'a, Pid, Arc<Process>>,
}

impl<'a> Iterator for ProcessGroupIter<'a> {
    type Item = &'a Arc<Process>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}
