// SPDX-License-Identifier: MPL-2.0

//! A unified PID table that maps numeric PIDs to threads, processes,
//! process groups, and sessions.
//!
//! In Linux, a single numeric PID can simultaneously identify a thread,
//! a process (thread group), a process group, and a session. The [`PidTable`]
//! captures all these associations in a single table, keyed by the numeric PID.
//!
//! Each entry in the table is a [`PidEntry`], which holds optional `Weak`
//! references to each type of entity associated with that PID.

use ostd::sync::RcuOption;

use super::{Pgid, Pid, Process, ProcessGroup, Session, Sid, posix_thread::AsPosixThread};
use crate::{
    events::{Events, Observer, Subject},
    prelude::*,
    thread::{Thread, Tid},
};

static PID_TABLE: Mutex<PidTable> = Mutex::new(PidTable::new());

/// An entry in the PID table that tracks all entities associated with a numeric PID.
///
/// A single numeric PID may simultaneously identify a thread, a process (thread group),
/// a process group, and a session. This struct captures all these associations via
/// [`RcuOption`]-protected `Arc` references, providing interior mutability.
struct PidEntry {
    thread: RcuOption<Arc<Thread>>,
    process: RcuOption<Arc<Process>>,
    process_group: RcuOption<Arc<ProcessGroup>>,
    session: RcuOption<Arc<Session>>,
}

impl PidEntry {
    /// Creates a new PID entry with empty associations.
    fn new() -> Self {
        Self {
            thread: RcuOption::new_none(),
            process: RcuOption::new_none(),
            process_group: RcuOption::new_none(),
            session: RcuOption::new_none(),
        }
    }

    /// Returns whether all associations are empty.
    fn is_empty(&self) -> bool {
        self.thread.read().is_none()
            && self.process.read().is_none()
            && self.process_group.read().is_none()
            && self.session.read().is_none()
    }
}

/// A unified table that maps numeric PIDs to their associated entities.
///
/// This table manages the associations between numeric PIDs and threads,
/// processes, process groups, and sessions.
pub struct PidTable {
    entries: BTreeMap<Pid, Arc<PidEntry>>,
    process_count: usize,
    subject: Subject<PidEvent>,
}

impl PidTable {
    /// Creates a new empty PID table.
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            process_count: 0,
            subject: Subject::new(),
        }
    }

    /// Removes the entry from the table if all its associations are empty.
    fn remove_if_empty(&mut self, pid: Pid) {
        if let Some(entry) = self.entries.get(&pid)
            && entry.is_empty()
        {
            self.entries.remove(&pid);
        }
    }

    // ==================== Thread operations ====================

    /// Returns the thread associated with the given TID.
    pub fn get_thread(&self, tid: Tid) -> Option<Arc<Thread>> {
        let entry = self.entries.get(&tid)?;
        let guard = entry.thread.read();
        guard.get().map(|r| Arc::clone(&r))
    }

    /// Adds a thread to the table.
    pub fn add_thread(&mut self, tid: Tid, thread: &Arc<Thread>) {
        let entry = self
            .entries
            .entry(tid)
            .or_insert_with(|| Arc::new(PidEntry::new()));
        entry.thread.update(Some(Arc::clone(thread)));
    }

    /// Removes the thread associated with the given TID.
    pub fn remove_thread(&mut self, tid: Tid) {
        if let Some(entry) = self.entries.get(&tid) {
            entry.thread.update(None);
        }
        self.remove_if_empty(tid);
    }

    /// Removes and returns the thread associated with the given TID.
    pub fn take_thread(&mut self, tid: Tid) -> Option<Arc<Thread>> {
        let entry = self.entries.get(&tid)?;
        let thread = {
            let guard = entry.thread.read();
            guard.get().map(|r| Arc::clone(&r))
        };
        if thread.is_some() {
            entry.thread.update(None);
        }
        self.remove_if_empty(tid);
        thread
    }

    /// Returns an iterator over all threads in the table.
    pub fn iter_threads(&self) -> impl Iterator<Item = Arc<Thread>> + '_ {
        self.entries
            .values()
            .filter_map(|entry| entry.thread.read().get().map(|r| Arc::clone(&r)))
    }

    // ==================== Process operations ====================

    /// Returns the process associated with the given PID.
    pub fn get_process(&self, pid: Pid) -> Option<Arc<Process>> {
        let entry = self.entries.get(&pid)?;
        let guard = entry.process.read();
        guard.get().map(|r| Arc::clone(&r))
    }

    /// Inserts a process into the table.
    pub fn insert_process(&mut self, pid: Pid, process: Arc<Process>) {
        let entry = self
            .entries
            .entry(pid)
            .or_insert_with(|| Arc::new(PidEntry::new()));
        entry.process.update(Some(process));
        self.process_count += 1;
    }

    /// Removes the process associated with the given PID and notifies observers.
    pub fn remove_process(&mut self, pid: Pid) {
        if let Some(entry) = self.entries.get(&pid) {
            entry.process.update(None);
            self.process_count -= 1;
            self.subject.notify_observers(&PidEvent::Exit(pid));
        }
        self.remove_if_empty(pid);
    }

    /// Returns the number of processes in the table.
    pub fn process_count(&self) -> usize {
        self.process_count
    }

    /// Returns an iterator over all processes in the table.
    pub fn iter_processes(&self) -> impl Iterator<Item = Arc<Process>> + '_ {
        self.entries
            .values()
            .filter_map(|entry| entry.process.read().get().map(|r| Arc::clone(&r)))
    }

    // ==================== Process group operations ====================

    /// Returns the process group associated with the given PGID.
    pub fn get_process_group(&self, pgid: Pgid) -> Option<Arc<ProcessGroup>> {
        let entry = self.entries.get(&pgid)?;
        let guard = entry.process_group.read();
        guard.get().map(|r| Arc::clone(&r))
    }

    /// Returns whether a process group with the given PGID exists.
    pub fn contains_process_group(&self, pgid: Pgid) -> bool {
        self.entries
            .get(&pgid)
            .is_some_and(|e| !e.process_group.read().is_none())
    }

    /// Inserts a process group into the table.
    pub fn insert_process_group(&mut self, pgid: Pgid, group: Arc<ProcessGroup>) {
        let entry = self
            .entries
            .entry(pgid)
            .or_insert_with(|| Arc::new(PidEntry::new()));
        entry.process_group.update(Some(group));
    }

    /// Removes the process group associated with the given PGID.
    pub fn remove_process_group(&mut self, pgid: Pgid) {
        if let Some(entry) = self.entries.get(&pgid) {
            entry.process_group.update(None);
        }
        self.remove_if_empty(pgid);
    }

    // ==================== Session operations ====================

    /// Returns the session associated with the given SID.
    pub fn get_session(&self, sid: Sid) -> Option<Arc<Session>> {
        let entry = self.entries.get(&sid)?;
        let guard = entry.session.read();
        guard.get().map(|r| Arc::clone(&r))
    }

    /// Inserts a session into the table.
    pub fn insert_session(&mut self, sid: Sid, session: Arc<Session>) {
        let entry = self
            .entries
            .entry(sid)
            .or_insert_with(|| Arc::new(PidEntry::new()));
        entry.session.update(Some(session));
    }

    /// Removes the session associated with the given SID.
    pub fn remove_session(&mut self, sid: Sid) {
        if let Some(entry) = self.entries.get(&sid) {
            entry.session.update(None);
        }
        self.remove_if_empty(sid);
    }

    // ==================== Observer operations ====================

    /// Registers an observer which watches [`PidEvent`].
    pub fn register_observer(&mut self, observer: Weak<dyn Observer<PidEvent>>) {
        self.subject.register_observer(observer);
    }

    /// Unregisters an observer which watches [`PidEvent`].
    pub fn unregister_observer(&mut self, observer: &Weak<dyn Observer<PidEvent>>) {
        self.subject.unregister_observer(observer);
    }
}

// ==================== Free functions (backward-compatible API) ====================

/// Returns a locked reference to the global PID table.
pub fn pid_table_mut() -> MutexGuard<'static, PidTable> {
    PID_TABLE.lock()
}

/// Locks the global PID table and applies the given function.
pub fn with_pid_table<F, R>(f: F) -> R
where
    F: FnOnce(&PidTable) -> R,
{
    let table = PID_TABLE.lock();
    f(&table)
}

// ************ Thread *************

/// Gets a thread with the given TID.
pub fn get_thread(tid: Tid) -> Option<Arc<Thread>> {
    PID_TABLE.lock().get_thread(tid)
}

/// Adds a POSIX thread to the global PID table.
pub fn add_thread(tid: Tid, thread: &Arc<Thread>) {
    debug_assert_eq!(tid, thread.as_posix_thread().unwrap().tid());
    PID_TABLE.lock().add_thread(tid, thread);
}

/// Removes a thread from the global PID table.
pub fn remove_thread(tid: Tid) {
    PID_TABLE.lock().remove_thread(tid);
}

/// Makes the current thread become the main thread if necessary.
pub(super) fn make_current_main_thread(ctx: &Context) {
    let pid = ctx.process.pid();
    let old_tid = ctx.posix_thread.tid();

    // The current thread is already the main thread.
    if old_tid == pid {
        return;
    }

    // The current thread is not the main thread.
    let mut tasks = ctx.process.tasks().lock();
    let mut pid_table = pid_table_mut();

    assert!(tasks.has_exited_main());
    assert!(tasks.in_execve());
    assert_eq!(tasks.as_slice().len(), 2);
    assert!(core::ptr::eq(ctx.task, tasks.as_slice()[1].as_ref()));

    tasks.swap_main(pid, old_tid);
    ctx.posix_thread.set_main(pid);

    pid_table.take_thread(pid).unwrap();
    let thread = pid_table.take_thread(old_tid).unwrap();
    pid_table.add_thread(pid, &thread);
}

// ************ Process *************

/// Gets a process with the given PID.
pub fn get_process(pid: Pid) -> Option<Arc<Process>> {
    PID_TABLE.lock().get_process(pid)
}

/// Returns the number of current processes.
pub fn process_count() -> usize {
    PID_TABLE.lock().process_count()
}

// ************ Process Group *************

/// Gets a process group with the given PGID.
pub fn get_process_group(pgid: &Pgid) -> Option<Arc<ProcessGroup>> {
    PID_TABLE.lock().get_process_group(*pgid)
}

// ************ Session *************

/// Gets a session with the given SID.
#[expect(dead_code)]
pub fn get_session(sid: &Sid) -> Option<Arc<Session>> {
    PID_TABLE.lock().get_session(*sid)
}

// ************ Observer *************

/// Registers an observer which watches [`PidEvent`].
pub fn register_observer(observer: Weak<dyn Observer<PidEvent>>) {
    PID_TABLE.lock().register_observer(observer);
}

/// Unregisters an observer which watches [`PidEvent`].
#[expect(dead_code)]
pub fn unregister_observer(observer: &Weak<dyn Observer<PidEvent>>) {
    PID_TABLE.lock().unregister_observer(observer);
}

#[derive(Copy, Clone)]
pub enum PidEvent {
    Exit(Pid),
}

impl Events for PidEvent {}
