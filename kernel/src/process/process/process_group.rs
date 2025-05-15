// SPDX-License-Identifier: MPL-2.0

use alloc::collections::btree_set::Iter;

use keyable_arc::KeyableArc;
use ostd::sync::PreemptDisabled;

use super::{Pgid, Process, Session};
use crate::{
    prelude::*,
    process::{pid_namespace::AncestorNsPids, signal::signals::Signal, PidNamespace},
};

/// A process group.
///
/// A process group represents a set of processes,
/// which has a unique identifier PGID (i.e., [`Pgid`]).
pub struct ProcessGroup {
    ns_pgids: AncestorNsPids,
    session: Weak<Session>,
    inner: SpinLock<Inner>,
}

struct Inner {
    processes: BTreeSet<KeyableArc<Process>>,
}

impl ProcessGroup {
    /// Creates a new process group with one process.
    ///
    /// The PGID is the same as the process ID, which means that the process will become the leader
    /// process of the new process group.
    ///
    /// The caller needs to ensure that the process does not belong to other process group.
    pub(super) fn new(process: Arc<Process>, session: Weak<Session>) -> Arc<Self> {
        let ns_pgids = process.ns_pids.clone();

        let inner = {
            let mut processes = BTreeSet::new();
            processes.insert(process.into());
            Inner { processes }
        };

        Arc::new(ProcessGroup {
            ns_pgids,
            session,
            inner: SpinLock::new(inner),
        })
    }

    /// Returns the process group identifier in the given PID namespace.
    ///
    /// If the process group is not visible in the namespace, this method will return `None`.
    pub fn pgid_in_ns(&self, pid_ns: &Arc<PidNamespace>) -> Option<Pgid> {
        pid_ns.get_current_id(&self.ns_pgids)
    }

    /// Returns the process group's identifier in all PID namespaces.
    pub fn ns_pgids(&self) -> &AncestorNsPids {
        &self.ns_pgids
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
        for process in self.inner.lock().processes.iter() {
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
    inner: SpinLockGuard<'a, Inner, PreemptDisabled>,
}

impl ProcessGroupGuard<'_> {
    /// Returns an iterator over the processes in the process group.
    pub fn iter(&self) -> ProcessGroupIter {
        ProcessGroupIter {
            inner: self.inner.processes.iter(),
        }
    }

    /// Inserts a process into the process group.
    ///
    /// The caller needs to ensure that the process didn't previously belong to the process group,
    /// but now does.
    pub(in crate::process) fn insert_process(&mut self, process: Arc<Process>) {
        let newly_added = self.inner.processes.insert(process.into());
        debug_assert!(newly_added);
    }

    /// Removes a process from the process group.
    ///
    /// The caller needs to ensure that the process previously belonged to the process group, but
    /// now doesn't.
    pub(in crate::process) fn remove_process(&mut self, process: &Arc<Process>) {
        let key = KeyableArc::from(process.clone());
        let is_removed = self.inner.processes.remove(&key);
        debug_assert!(is_removed);
    }

    /// Returns whether the process group is empty.
    pub(in crate::process) fn is_empty(&self) -> bool {
        self.inner.processes.is_empty()
    }
}

/// An iterator over the processes of the process group.
pub struct ProcessGroupIter<'a> {
    inner: Iter<'a, KeyableArc<Process>>,
}

impl<'a> Iterator for ProcessGroupIter<'a> {
    type Item = Arc<Process>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|key| key.clone().into())
    }
}
