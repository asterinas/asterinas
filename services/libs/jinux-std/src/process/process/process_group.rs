use super::{Pgid, Pid, Process, Session};
use crate::prelude::*;
use crate::process::signal::signals::Signal;

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

        if let Some(leader) = &self.leader && Arc::ptr_eq(leader, &process) {
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

    /// Broadcasts signal to all processes in the group.
    pub fn broadcast_signal(&self, signal: impl Signal + Clone + 'static) {
        for process in self.inner.lock().processes.values() {
            process.enqueue_signal(Box::new(signal.clone()));
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
