// SPDX-License-Identifier: MPL-2.0

use alloc::collections::btree_map::Values;

use super::{Pgid, Process, Session};
use crate::{
    prelude::*,
    process::{KernelPid, PidChain, PidNamespace, signal::signals::Signal},
};

/// A process group.
pub struct ProcessGroup {
    owner_pid_ns: Arc<PidNamespace>,
    kernel_pgid: KernelPid,
    pgid_chain: PidChain,
    leader: Weak<Process>,
    session: Arc<Session>,
    inner: Mutex<Inner>,
}

struct Inner {
    processes: BTreeMap<KernelPid, Weak<Process>>,
}

impl ProcessGroup {
    /// Creates a new process group with one process.
    pub(super) fn new(process: Arc<Process>, session: Arc<Session>) -> Arc<Self> {
        let kernel_pgid = process.kernel_pid();
        let inner = {
            let mut processes = BTreeMap::new();
            processes.insert(kernel_pgid, Arc::downgrade(&process));
            Inner { processes }
        };

        Arc::new(Self {
            owner_pid_ns: process.active_pid_ns().clone(),
            kernel_pgid,
            pgid_chain: process.pid_chain().clone(),
            leader: Arc::downgrade(&process),
            session,
            inner: Mutex::new(inner),
        })
    }

    #[expect(dead_code)]
    pub fn owner_pid_ns(&self) -> &Arc<PidNamespace> {
        &self.owner_pid_ns
    }

    pub fn kernel_pgid(&self) -> KernelPid {
        self.kernel_pgid
    }

    #[expect(dead_code)]
    pub fn leader(&self) -> Option<Arc<Process>> {
        self.leader.upgrade()
    }

    pub fn session(&self) -> &Arc<Session> {
        &self.session
    }

    pub fn pgid(&self) -> Pgid {
        self.canonical_pgid_unchecked()
    }

    pub fn pgid_in(&self, ns: &PidNamespace) -> Option<Pgid> {
        self.pgid_chain.nr_in(ns)
    }

    pub fn pgid_chain(&self) -> &PidChain {
        &self.pgid_chain
    }

    pub(super) fn canonical_pgid_unchecked(&self) -> Pgid {
        self.pgid_chain.active_link().nr()
    }

    pub fn lock(&self) -> ProcessGroupGuard<'_> {
        ProcessGroupGuard {
            inner: self.inner.lock(),
        }
    }

    pub fn broadcast_signal(&self, signal: impl Signal + Clone + 'static) {
        for process in self
            .inner
            .lock()
            .processes
            .values()
            .filter_map(Weak::upgrade)
        {
            process.enqueue_signal(Box::new(signal.clone()));
        }
    }
}

#[clippy::has_significant_drop]
#[must_use]
pub struct ProcessGroupGuard<'a> {
    inner: MutexGuard<'a, Inner>,
}

impl ProcessGroupGuard<'_> {
    pub fn iter(&self) -> ProcessGroupIter<'_> {
        ProcessGroupIter {
            inner: self.inner.processes.values(),
        }
    }

    pub(in crate::process) fn insert_process(&mut self, process: Arc<Process>) {
        let old_process = self
            .inner
            .processes
            .insert(process.kernel_pid(), Arc::downgrade(&process));
        debug_assert!(old_process.is_none());
    }

    pub(in crate::process) fn remove_process(&mut self, pid: &KernelPid) {
        let process = self.inner.processes.remove(pid);
        debug_assert!(process.is_some());
    }

    pub(in crate::process) fn is_empty(&self) -> bool {
        self.inner.processes.is_empty()
    }
}

pub struct ProcessGroupIter<'a> {
    inner: Values<'a, KernelPid, Weak<Process>>,
}

impl<'a> Iterator for ProcessGroupIter<'a> {
    type Item = Arc<Process>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.by_ref().find_map(Weak::upgrade)
    }
}
