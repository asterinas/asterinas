// SPDX-License-Identifier: MPL-2.0

use super::{Pgid, Process, ProcessGroup, Sid, Terminal};
use crate::prelude::*;

/// A session.
///
/// A session is a collection of related process groups, which has a unique identifier SID (i.e.,
/// [`Sid`]). Process groups and sessions form a two-level hierarchical relationship between
/// processes.
///
/// **Leader**: A *session leader* is the process that creates the session and whose process ID is
/// equal to the session ID.
///
/// **Controlling terminal**: A terminal can be used to manage all processes in the session. The
/// controlling terminal is established when the session leader first opens a terminal.
pub struct Session {
    sid: Sid,
    inner: Mutex<Inner>,
}

struct Inner {
    process_groups: BTreeMap<Pgid, Arc<ProcessGroup>>,
    terminal: Option<Arc<dyn Terminal>>,
}

impl Session {
    /// Creates a new session and a new process group with one process.
    ///
    /// The SID and the PGID are the same as the process ID, which means that the process will
    /// become the leader process of the new session and the new process group.
    ///
    /// The caller needs to ensure that the process does not belong to other process group or other
    /// session.
    pub(in crate::process) fn new_pair(process: Arc<Process>) -> (Arc<Self>, Arc<ProcessGroup>) {
        let mut process_group = None;

        let session = Arc::new_cyclic(|weak_session| {
            let group = ProcessGroup::new(process, weak_session.clone());
            process_group = Some(group.clone());

            let pgid = group.pgid();

            let inner = {
                let mut process_groups = BTreeMap::new();
                process_groups.insert(pgid, group);
                Inner {
                    process_groups,
                    terminal: None,
                }
            };

            Self {
                sid: pgid,
                inner: Mutex::new(inner),
            }
        });

        (session, process_group.unwrap())
    }

    /// Returns the session identifier.
    pub fn sid(&self) -> Sid {
        self.sid
    }

    /// Returns whether the process is the session leader.
    pub(super) fn is_leader(&self, process: &Process) -> bool {
        self.sid == process.pid()
    }

    /// Acquires a lock on the session.
    pub fn lock(&self) -> SessionGuard {
        SessionGuard {
            inner: self.inner.lock(),
        }
    }
}

/// A scoped lock guard for a session.
///
/// It provides some public methods to prevent the exposure of the inner type.
#[clippy::has_significant_drop]
#[must_use]
pub struct SessionGuard<'a> {
    inner: MutexGuard<'a, Inner>,
}

impl SessionGuard<'_> {
    /// Sets the controlling terminal of the session.
    ///
    /// The caller needs to ensure that the job control of the old and new controlling terminals
    /// are properly updated and synchronized.
    pub(super) fn set_terminal(&mut self, terminal: Option<Arc<dyn Terminal>>) {
        self.inner.terminal = terminal;
    }

    /// Returns the controlling terminal of the session.
    pub fn terminal(&self) -> Option<&Arc<dyn Terminal>> {
        self.inner.terminal.as_ref()
    }

    /// Inserts a process group into the session.
    ///
    /// The caller needs to ensure that the process group didn't previously belong to the session,
    /// but now does.
    pub(in crate::process) fn insert_process_group(&mut self, process_group: Arc<ProcessGroup>) {
        let old_process_group = self
            .inner
            .process_groups
            .insert(process_group.pgid(), process_group);
        debug_assert!(old_process_group.is_none());
    }

    /// Removes a process group from the session.
    ///
    /// The caller needs to ensure that the process group previously belonged to the session, but
    /// now doesn't.
    pub(in crate::process) fn remove_process_group(&mut self, pgid: &Pgid) {
        self.inner.process_groups.remove(pgid);
    }

    /// Returns whether the session is empty.
    pub(in crate::process) fn is_empty(&self) -> bool {
        self.inner.process_groups.is_empty()
    }
}
