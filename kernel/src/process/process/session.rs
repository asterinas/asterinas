// SPDX-License-Identifier: MPL-2.0

use super::{Process, ProcessGroup, Terminal};
use crate::{
    prelude::*,
    process::{KernelPid, PidChain, PidNamespace, Sid},
};

/// A session.
pub struct Session {
    owner_pid_ns: Arc<PidNamespace>,
    kernel_sid: KernelPid,
    sid_chain: PidChain,
    leader: Weak<Process>,
    inner: Mutex<Inner>,
}

struct Inner {
    process_groups: BTreeMap<KernelPid, Weak<ProcessGroup>>,
    terminal: Option<Arc<dyn Terminal>>,
}

impl Session {
    pub(in crate::process) fn new_pair(process: Arc<Process>) -> (Arc<Self>, Arc<ProcessGroup>) {
        let session = Arc::new(Self {
            owner_pid_ns: process.active_pid_ns().clone(),
            kernel_sid: process.kernel_pid(),
            sid_chain: process.pid_chain().clone(),
            leader: Arc::downgrade(&process),
            inner: Mutex::new(Inner {
                process_groups: BTreeMap::new(),
                terminal: None,
            }),
        });

        let process_group = ProcessGroup::new(process, session.clone());
        session
            .inner
            .lock()
            .process_groups
            .insert(process_group.kernel_pgid(), Arc::downgrade(&process_group));

        (session, process_group)
    }

    #[expect(dead_code)]
    pub fn owner_pid_ns(&self) -> &Arc<PidNamespace> {
        &self.owner_pid_ns
    }

    #[expect(dead_code)]
    pub fn kernel_sid(&self) -> KernelPid {
        self.kernel_sid
    }

    #[expect(dead_code)]
    pub fn leader(&self) -> Option<Arc<Process>> {
        self.leader.upgrade()
    }

    pub fn sid(&self) -> Sid {
        self.canonical_sid_unchecked()
    }

    pub fn sid_in(&self, ns: &PidNamespace) -> Option<Sid> {
        self.sid_chain.nr_in(ns)
    }

    pub fn sid_chain(&self) -> &PidChain {
        &self.sid_chain
    }

    pub(super) fn canonical_sid_unchecked(&self) -> Sid {
        self.sid_chain.active_link().nr()
    }

    pub(super) fn is_leader(&self, process: &Process) -> bool {
        self.kernel_sid == process.kernel_pid()
    }

    pub fn lock(&self) -> SessionGuard<'_> {
        SessionGuard {
            inner: self.inner.lock(),
        }
    }
}

#[clippy::has_significant_drop]
#[must_use]
pub struct SessionGuard<'a> {
    inner: MutexGuard<'a, Inner>,
}

impl SessionGuard<'_> {
    pub(super) fn set_terminal(&mut self, terminal: Option<Arc<dyn Terminal>>) {
        self.inner.terminal = terminal;
    }

    pub fn terminal(&self) -> Option<&Arc<dyn Terminal>> {
        self.inner.terminal.as_ref()
    }

    pub(in crate::process) fn insert_process_group(&mut self, process_group: Arc<ProcessGroup>) {
        let old_process_group = self
            .inner
            .process_groups
            .insert(process_group.kernel_pgid(), Arc::downgrade(&process_group));
        debug_assert!(old_process_group.is_none());
    }

    pub(in crate::process) fn remove_process_group(&mut self, pgid: &KernelPid) {
        self.inner.process_groups.remove(pgid);
    }

    pub(in crate::process) fn is_empty(&self) -> bool {
        self.inner.process_groups.is_empty()
    }
}
