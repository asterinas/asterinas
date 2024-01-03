// SPDX-License-Identifier: MPL-2.0

//! A global table stores the pid to process mapping.
//! This table can be used to get process with pid.
//! TODO: progress group, thread all need similar mapping

use crate::events::{Events, Observer, Subject};
use crate::prelude::*;

use super::{Pgid, Pid, Process, ProcessGroup, Session, Sid};

static PROCESS_TABLE: Mutex<BTreeMap<Pid, Arc<Process>>> = Mutex::new(BTreeMap::new());
static PROCESS_GROUP_TABLE: Mutex<BTreeMap<Pgid, Arc<ProcessGroup>>> = Mutex::new(BTreeMap::new());
static PROCESS_TABLE_SUBJECT: Subject<PidEvent> = Subject::new();
static SESSION_TABLE: Mutex<BTreeMap<Sid, Arc<Session>>> = Mutex::new(BTreeMap::new());

// ************ Process *************

/// Gets a process with pid
pub fn get_process(pid: &Pid) -> Option<Arc<Process>> {
    PROCESS_TABLE.lock().get(pid).cloned()
}

pub(super) fn process_table_mut() -> MutexGuard<'static, BTreeMap<Pid, Arc<Process>>> {
    PROCESS_TABLE.lock()
}

/// Gets all processes
pub fn get_all_processes() -> Vec<Arc<Process>> {
    PROCESS_TABLE
        .lock()
        .iter()
        .map(|(_, process)| process.clone())
        .collect()
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
    PROCESS_TABLE_SUBJECT.register_observer(observer, ());
}

/// Unregisters an observer which watches `PidEvent`.
pub fn unregister_observer(observer: &Weak<dyn Observer<PidEvent>>) {
    PROCESS_TABLE_SUBJECT.unregister_observer(observer);
}

#[derive(Copy, Clone)]
pub enum PidEvent {
    Exit(Pid),
}

impl Events for PidEvent {}
