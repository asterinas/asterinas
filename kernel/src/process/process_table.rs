// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

//! A global table stores the pid to process mapping.
//! This table can be used to get process with pid.
//! TODO: progress group, thread all need similar mapping

use alloc::collections::btree_map::Values;

use super::{Pgid, Pid, Process, ProcessGroup, Session, Sid};
use crate::{
    events::{Events, Observer, Subject},
    prelude::*,
};

static PROCESS_TABLE: Mutex<ProcessTable> = Mutex::new(ProcessTable::new());
static PROCESS_GROUP_TABLE: Mutex<BTreeMap<Pgid, Arc<ProcessGroup>>> = Mutex::new(BTreeMap::new());
static SESSION_TABLE: Mutex<BTreeMap<Sid, Arc<Session>>> = Mutex::new(BTreeMap::new());

// ************ Process *************
/// Gets a process with pid
pub fn get_process(pid: Pid) -> Option<Arc<Process>> {
    PROCESS_TABLE.lock().get(pid).cloned()
}

pub fn process_table_mut() -> MutexGuard<'static, ProcessTable> {
    PROCESS_TABLE.lock()
}

/// Process Table.
pub struct ProcessTable {
    inner: BTreeMap<Pid, Arc<Process>>,
    subject: Subject<PidEvent>,
}

impl ProcessTable {
    pub const fn new() -> Self {
        Self {
            inner: BTreeMap::new(),
            subject: Subject::new(),
        }
    }

    pub fn get(&self, pid: Pid) -> Option<&Arc<Process>> {
        self.inner.get(&pid)
    }

    pub fn insert(&mut self, pid: Pid, process: Arc<Process>) {
        self.inner.insert(pid, process);
    }

    pub fn remove(&mut self, pid: Pid) {
        self.inner.remove(&pid);
        self.subject.notify_observers(&PidEvent::Exit(pid));
    }

    /// Returns an iterator over the processes in the table.
    pub fn iter(&self) -> ProcessTableIter {
        ProcessTableIter {
            inner: self.inner.values(),
        }
    }

    /// Registers an observer which watches `PidEvent`.
    pub fn register_observer(&self, observer: Weak<dyn Observer<PidEvent>>) {
        self.subject.register_observer(observer, ());
    }

    /// Unregisters an observer which watches `PidEvent`.
    pub fn unregister_observer(&self, observer: &Weak<dyn Observer<PidEvent>>) {
        self.subject.unregister_observer(observer);
    }
}

/// An iterator over the processes of the process table.
pub struct ProcessTableIter<'a> {
    inner: Values<'a, Pid, Arc<Process>>,
}

impl<'a> Iterator for ProcessTableIter<'a> {
    type Item = &'a Arc<Process>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

// ************ Process Group *************

/// Gets a process group with `pgid`
pub fn get_process_group(pgid: &Pgid) -> Option<Arc<ProcessGroup>> {
    PROCESS_GROUP_TABLE.lock().get(pgid).cloned()
}

/// Returns whether process table contains process group with pgid
pub fn contain_process_group(pgid: &Pgid) -> bool {
    PROCESS_GROUP_TABLE.lock().contains_key(pgid)
}

pub(super) fn group_table_mut() -> MutexGuard<'static, BTreeMap<Pgid, Arc<ProcessGroup>>> {
    PROCESS_GROUP_TABLE.lock()
}

// ************ Session *************

/// Gets a session with `sid`.
pub fn get_session(sid: &Sid) -> Option<Arc<Session>> {
    SESSION_TABLE.lock().get(sid).map(Arc::clone)
}

pub(super) fn session_table_mut() -> MutexGuard<'static, BTreeMap<Sid, Arc<Session>>> {
    SESSION_TABLE.lock()
}

// ************ Observer *************

/// Registers an observer which watches `PidEvent`.
pub fn register_observer(observer: Weak<dyn Observer<PidEvent>>) {
    PROCESS_TABLE.lock().register_observer(observer);
}

/// Unregisters an observer which watches `PidEvent`.
pub fn unregister_observer(observer: &Weak<dyn Observer<PidEvent>>) {
    PROCESS_TABLE.lock().unregister_observer(observer);
}

#[derive(Copy, Clone)]
pub enum PidEvent {
    Exit(Pid),
}

impl Events for PidEvent {}
