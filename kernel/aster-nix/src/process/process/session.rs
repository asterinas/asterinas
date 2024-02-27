// SPDX-License-Identifier: MPL-2.0

use super::{Pgid, Process, ProcessGroup, Sid, Terminal};
use crate::prelude::*;

/// A `Session` is a collection of related process groups. Each session has a
/// unique identifier `sid`. Process groups and sessions form a two-level
/// hierarchical relationship between processes.
///
/// **Leader**: A *session leader* is the process that creates a new session and whose process
/// ID becomes the session ID.
///
/// **Controlling terminal**: The terminal can be used to manage all processes in the session. The
/// controlling terminal is established when the session leader first opens a terminal.
pub struct Session {
    sid: Sid,
    pub(in crate::process) inner: Mutex<Inner>,
}

pub(in crate::process) struct Inner {
    pub(in crate::process) process_groups: BTreeMap<Pgid, Arc<ProcessGroup>>,
    pub(in crate::process) leader: Option<Arc<Process>>,
    pub(in crate::process) terminal: Option<Arc<dyn Terminal>>,
}

impl Inner {
    pub(in crate::process) fn is_empty(&self) -> bool {
        self.process_groups.is_empty()
    }

    pub(in crate::process) fn remove_process(&mut self, process: &Arc<Process>) {
        if let Some(leader) = &self.leader
            && Arc::ptr_eq(leader, process)
        {
            self.leader = None;
        }
    }

    pub(in crate::process) fn remove_process_group(&mut self, pgid: &Pgid) {
        self.process_groups.remove(pgid);
    }
}

impl Session {
    /// Creates a new session for the process group. The process group becomes the member of
    /// the new session.
    ///
    /// The caller needs to ensure that the group does not belong to any session, and the caller
    /// should set the leader process after creating the session.
    pub(in crate::process) fn new(group: Arc<ProcessGroup>) -> Arc<Self> {
        let sid = group.pgid();
        let inner = {
            let mut process_groups = BTreeMap::new();
            process_groups.insert(group.pgid(), group);

            Inner {
                process_groups,
                leader: None,
                terminal: None,
            }
        };
        Arc::new(Self {
            sid,
            inner: Mutex::new(inner),
        })
    }

    /// Returns the session id
    pub fn sid(&self) -> Sid {
        self.sid
    }

    /// Returns the leader process.
    pub fn leader(&self) -> Option<Arc<Process>> {
        self.inner.lock().leader.clone()
    }

    /// Returns whether `self` contains the `process_group`
    pub(in crate::process) fn contains_process_group(
        self: &Arc<Self>,
        process_group: &Arc<ProcessGroup>,
    ) -> bool {
        self.inner
            .lock()
            .process_groups
            .contains_key(&process_group.pgid())
    }

    /// Sets terminal as the controlling terminal of the session. The `get_terminal` method
    /// should set the session for the terminal and returns the session.
    ///
    /// If the session already has controlling terminal, this method will return `Err(EPERM)`.
    pub fn set_terminal<F>(&self, get_terminal: F) -> Result<()>
    where
        F: Fn() -> Result<Arc<dyn Terminal>>,
    {
        let mut inner = self.inner.lock();

        if inner.terminal.is_some() {
            return_errno_with_message!(
                Errno::EPERM,
                "current session already has controlling terminal"
            );
        }

        let terminal = get_terminal()?;
        inner.terminal = Some(terminal);
        Ok(())
    }

    /// Releases the controlling terminal of the session.
    ///
    /// If the session does not have controlling terminal, this method will return `ENOTTY`.
    pub fn release_terminal<F>(&self, release_session: F) -> Result<()>
    where
        F: Fn(&Arc<dyn Terminal>) -> Result<()>,
    {
        let mut inner = self.inner.lock();
        if inner.terminal.is_none() {
            return_errno_with_message!(
                Errno::ENOTTY,
                "current session does not has controlling terminal"
            );
        }

        let terminal = inner.terminal.as_ref().unwrap();
        release_session(terminal)?;
        inner.terminal = None;
        Ok(())
    }

    /// Returns the controlling terminal of `self`.
    pub fn terminal(&self) -> Option<Arc<dyn Terminal>> {
        self.inner.lock().terminal.clone()
    }
}
