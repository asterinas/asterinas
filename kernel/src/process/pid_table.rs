// SPDX-License-Identifier: MPL-2.0

//! A unified PID table that maps numeric identifiers to threads, processes,
//! process groups, and sessions.
//!
//! This design is inspired by Linux's `struct pid`. Each [`PidEntry`] tracks
//! the kernel objects that share the same numeric identifier, which eliminates
//! the need for separate per-type lookup tables.

use super::{Pgid, Pid, Process, ProcessGroup, Session, Sid};
use crate::{
    events::{Events, Observer, Subject},
    prelude::*,
    process::posix_thread::AsPosixThread,
    thread::{Thread, Tid},
};

static PID_TABLE: Mutex<PidTable> = Mutex::new(PidTable::new());

/// An entry in the unified PID table.
///
/// Each entry stores references to the thread, process, process group, and
/// session that share the same numeric identifier. Not all slots need to be
/// occupied at the same time.
struct PidEntry {
    inner: Mutex<PidEntryInner>,
}

struct PidEntryInner {
    thread: Weak<Thread>,
    process: Weak<Process>,
    process_group: Weak<ProcessGroup>,
    session: Weak<Session>,
}

impl PidEntry {
    /// Creates a new empty `PidEntry`.
    fn new() -> Self {
        Self {
            inner: Mutex::new(PidEntryInner {
                thread: Weak::new(),
                process: Weak::new(),
                process_group: Weak::new(),
                session: Weak::new(),
            }),
        }
    }

    /// Returns the thread associated with the entry, if any.
    fn thread(&self) -> Option<Arc<Thread>> {
        self.inner.lock().thread.upgrade()
    }

    /// Returns the process associated with the entry, if any.
    fn process(&self) -> Option<Arc<Process>> {
        self.inner.lock().process.upgrade()
    }

    /// Returns whether the entry still tracks a live process.
    fn has_live_process(&self) -> bool {
        self.inner.lock().process.strong_count() > 0
    }

    /// Returns the process group associated with the entry, if any.
    fn process_group(&self) -> Option<Arc<ProcessGroup>> {
        self.inner.lock().process_group.upgrade()
    }

    /// Sets the thread reference.
    pub(super) fn set_thread(&self, thread: &Arc<Thread>) {
        self.inner.lock().thread = Arc::downgrade(thread);
    }

    /// Clears the thread reference.
    pub(super) fn clear_thread(&self) {
        self.inner.lock().thread = Weak::new();
    }

    /// Sets the process reference.
    pub(super) fn set_process(&self, process: &Arc<Process>) {
        self.inner.lock().process = Arc::downgrade(process);
    }

    /// Clears the process reference.
    pub(super) fn clear_process(&self) {
        self.inner.lock().process = Weak::new();
    }

    /// Sets the process group reference.
    pub(super) fn set_process_group(&self, group: &Arc<ProcessGroup>) {
        self.inner.lock().process_group = Arc::downgrade(group);
    }

    /// Clears the process group reference.
    pub(super) fn clear_process_group(&self) {
        self.inner.lock().process_group = Weak::new();
    }

    /// Sets the session reference.
    pub(super) fn set_session(&self, session: &Arc<Session>) {
        self.inner.lock().session = Arc::downgrade(session);
    }

    /// Clears the session reference.
    pub(super) fn clear_session(&self) {
        self.inner.lock().session = Weak::new();
    }

    /// Returns `true` if the entry no longer tracks any live object.
    fn is_empty(&self) -> bool {
        let inner = self.inner.lock();
        inner.thread.strong_count() == 0
            && inner.process.strong_count() == 0
            && inner.process_group.strong_count() == 0
            && inner.session.strong_count() == 0
    }
}

/// The unified PID table.
///
/// Replaces the former separate process, process-group, session, and thread
/// tables with a single structure.
pub(crate) struct PidTable {
    entries: BTreeMap<u32, Arc<PidEntry>>,
    process_count: usize,
    subject: Subject<PidEvent>,
}

impl PidTable {
    const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            process_count: 0,
            subject: Subject::new(),
        }
    }

    /// Returns the entry for the given id, or creates a new one if absent.
    fn get_or_create_entry(&mut self, id: u32) -> Arc<PidEntry> {
        self.entries
            .entry(id)
            .or_insert_with(|| Arc::new(PidEntry::new()))
            .clone()
    }

    /// Removes the entry entirely if all weak references are dead.
    fn try_remove_entry(&mut self, id: u32) {
        if let Some(entry) = self.entries.get(&id)
            && entry.is_empty()
        {
            self.entries.remove(&id);
        }
    }

    // ---- Thread operations ----

    /// Inserts a thread into the table.
    pub fn insert_thread(&mut self, tid: Tid, thread: Arc<Thread>) {
        debug_assert_eq!(tid, thread.as_posix_thread().unwrap().tid());
        let entry = self.get_or_create_entry(tid);
        entry.set_thread(&thread);
    }

    /// Removes a thread from the table.
    pub fn remove_thread(&mut self, tid: Tid) {
        if let Some(entry) = self.entries.get(&tid) {
            entry.clear_thread();
        }
        self.try_remove_entry(tid);
    }

    /// Gets a thread by tid.
    pub fn get_thread(&self, tid: Tid) -> Option<Arc<Thread>> {
        self.entries.get(&tid).and_then(|e| e.thread())
    }

    // ---- Process operations ----

    /// Inserts a process into the table.
    pub fn insert_process(&mut self, pid: Pid, process: Arc<Process>) {
        let entry = self.get_or_create_entry(pid);
        if !entry.has_live_process() {
            self.process_count += 1;
        }
        entry.set_process(&process);
    }

    /// Removes a process from the table and notifies observers.
    pub fn remove_process(&mut self, pid: Pid) {
        let Some(entry) = self.entries.get(&pid) else {
            return;
        };

        if !entry.has_live_process() {
            return;
        }

        self.process_count -= 1;
        entry.clear_process();
        self.subject.notify_observers(&PidEvent::Exit(pid));
        self.try_remove_entry(pid);
    }

    /// Gets a process by pid.
    pub fn get_process(&self, pid: Pid) -> Option<Arc<Process>> {
        self.entries.get(&pid).and_then(|e| e.process())
    }

    /// Returns an iterator over processes that have a live process reference.
    pub fn iter_processes(&self) -> impl Iterator<Item = Arc<Process>> + '_ {
        self.entries.values().filter_map(|e| e.process())
    }

    /// Returns the number of live processes.
    pub fn process_count(&self) -> usize {
        self.process_count
    }

    // ---- Process group operations ----

    /// Inserts a process group into the table.
    pub fn insert_process_group(&mut self, pgid: Pgid, group: Arc<ProcessGroup>) {
        let entry = self.get_or_create_entry(pgid);
        entry.set_process_group(&group);
    }

    /// Removes a process group from the table.
    pub fn remove_process_group(&mut self, pgid: Pgid) {
        if let Some(entry) = self.entries.get(&pgid) {
            entry.clear_process_group();
        }
        self.try_remove_entry(pgid);
    }

    /// Gets a process group by pgid.
    pub fn get_process_group(&self, pgid: &Pgid) -> Option<Arc<ProcessGroup>> {
        self.entries.get(pgid).and_then(|e| e.process_group())
    }

    /// Returns whether a process group with the given pgid exists.
    pub fn contains_process_group(&self, pgid: &Pgid) -> bool {
        self.entries
            .get(pgid)
            .is_some_and(|entry| entry.inner.lock().process_group.strong_count() > 0)
    }

    // ---- Session operations ----

    /// Inserts a session into the table.
    pub fn insert_session(&mut self, sid: Sid, session: Arc<Session>) {
        let entry = self.get_or_create_entry(sid);
        entry.set_session(&session);
    }

    /// Removes a session from the table.
    pub fn remove_session(&mut self, sid: Sid) {
        if let Some(entry) = self.entries.get(&sid) {
            entry.clear_session();
        }
        self.try_remove_entry(sid);
    }

    /// Returns an iterator over threads that have a live thread reference.
    pub fn iter_threads(&self) -> impl Iterator<Item = Arc<Thread>> + '_ {
        self.entries.values().filter_map(|entry| entry.thread())
    }

    // ---- Observer operations ----

    /// Registers an observer which watches `PidEvent`.
    pub fn register_observer(&mut self, observer: Weak<dyn Observer<PidEvent>>) {
        self.subject.register_observer(observer);
    }
}

/// Acquires a mutable reference to the global PID table.
pub(crate) fn pid_table_mut() -> MutexGuard<'static, PidTable> {
    PID_TABLE.lock()
}

/// Makes the current thread become the main thread if necessary.
pub(in crate::process) fn make_current_main_thread(ctx: &Context) {
    let pid = ctx.process.pid();
    let old_tid = ctx.posix_thread.tid();

    if old_tid == pid {
        return;
    }

    // Lock order: pid table -> tasks of process.
    let mut pid_table = pid_table_mut();
    let mut tasks = ctx.process.tasks().lock();

    assert!(tasks.has_exited_main());
    assert!(tasks.in_execve());
    assert_eq!(tasks.as_slice().len(), 2);
    assert!(core::ptr::eq(ctx.task, tasks.as_slice()[1].as_ref()));

    tasks.swap_main(pid, old_tid);
    ctx.posix_thread.set_main(pid);

    pid_table.remove_thread(pid);
    let thread = pid_table.get_thread(old_tid).unwrap();
    pid_table.remove_thread(old_tid);
    pid_table.insert_thread(pid, thread);
}

/// An event emitted when a process exits.
#[derive(Copy, Clone)]
pub(crate) enum PidEvent {
    /// A process with the given PID has exited.
    Exit(Pid),
}

impl Events for PidEvent {}
