// SPDX-License-Identifier: MPL-2.0

use alloc::collections::btree_map::Values;

use super::{Pgid, Pid, Process, Session};
use crate::{prelude::*, process::signal::signals::Signal};

/// A process group.
///
/// A process group represents a set of processes,
/// which has a unique identifier PGID (i.e., [`Pgid`]).
pub struct ProcessGroup {
    pgid: Pgid,
    session: Weak<Session>,
    inner: Mutex<Inner>,
}

struct Inner {
    processes: BTreeMap<Pid, Arc<Process>>,
}

impl ProcessGroup {
    /// Creates a new process group with one process.
    ///
    /// The PGID is the same as the process ID, which means that the process will become the leader
    /// process of the new process group.
    ///
    /// The caller needs to ensure that the process does not belong to other process group.
    pub(super) fn new(process: Arc<Process>, session: Weak<Session>) -> Arc<Self> {
        let pid = process.pid();

        let inner = {
            let mut processes = BTreeMap::new();
            processes.insert(pid, process);
            Inner { processes }
        };

        Arc::new(ProcessGroup {
            pgid: pid,
            session,
            inner: Mutex::new(inner),
        })
    }

    /// Returns the process group identifier.
    pub fn pgid(&self) -> Pgid {
        self.pgid
    }

    /// Returns the session to which the process group belongs.
    pub fn session(&self) -> Option<Arc<Session>> {
        self.session.upgrade()
    }

    /// Acquires a lock on the process group.
    pub fn lock(&self) -> ProcessGroupGuard {
        ProcessGroupGuard {
            inner: self.inner.lock(),
        }
    }

    /// Broadcasts the signal to all processes in the process group.
    ///
    /// This method should only be used to broadcast fault signals and kernel signals.
    //
    // TODO: Do some checks to forbid user signals.
    pub fn broadcast_signal(&self, signal: impl Signal + Clone + 'static) {
        for process in self.inner.lock().processes.values() {
            process.enqueue_signal(signal.clone());
        }
    }
}

/// A scoped lock guard for a process group.
///
/// It provides some public methods to prevent the exposure of the inner type.
#[clippy::has_significant_drop]
#[must_use]
pub struct ProcessGroupGuard<'a> {
    inner: MutexGuard<'a, Inner>,
}

impl ProcessGroupGuard<'_> {
    /// Returns an iterator over the processes in the process group.
    pub fn iter(&self) -> ProcessGroupIter {
        ProcessGroupIter {
            inner: self.inner.processes.values(),
        }
    }

    /// Inserts a process into the process group.
    ///
    /// The caller needs to ensure that the process didn't previously belong to the process group,
    /// but now does.
    pub(in crate::process) fn insert_process(&mut self, process: Arc<Process>) {
        let old_process = self.inner.processes.insert(process.pid(), process);
        debug_assert!(old_process.is_none());
    }

    /// Removes a process from the process group.
    ///
    /// The caller needs to ensure that the process previously belonged to the process group, but
    /// now doesn't.
    pub(in crate::process) fn remove_process(&mut self, pid: &Pid) {
        let process = self.inner.processes.remove(pid);
        debug_assert!(process.is_some());
    }

    /// Returns whether the process group is empty.
    pub(in crate::process) fn is_empty(&self) -> bool {
        self.inner.processes.is_empty()
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
