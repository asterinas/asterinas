// SPDX-License-Identifier: MPL-2.0

//! A unified PID table that maps numeric identifiers to threads, processes,
//! process groups, and sessions.
//!
//! This design is inspired by Linux's `struct pid`. Each [`PidEntry`] tracks
//! the kernel objects that share the same numeric identifier, which eliminates
//! the need for separate per-type lookup tables.

use alloc::collections::btree_map::Entry;

use super::{Pgid, Pid, Process, ProcessGroup, Session, Sid};
use crate::{
    prelude::*,
    process::posix_thread::AsPosixThread,
    thread::{Thread, Tid},
};

static PID_TABLE: Mutex<PidTable> = Mutex::new(PidTable::new());

/// The unified PID table.
///
/// Combines the process, process-group, session, and thread tables into a
/// single structure.
pub struct PidTable {
    entries: BTreeMap<u32, Arc<PidEntry>>,
    process_count: usize,
}

impl PidTable {
    const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            process_count: 0,
        }
    }

    /// Returns the entry for the given ID, or creates a new one if absent.
    fn get_or_create_entry(&mut self, id: u32) -> &Arc<PidEntry> {
        self.entries
            .entry(id)
            .or_insert_with(|| Arc::new(PidEntry::new()))
    }

    // ---- Thread operations ----

    /// Inserts a thread into the table.
    pub(super) fn insert_thread(&mut self, tid: Tid, thread: &Arc<Thread>) {
        debug_assert_eq!(tid, thread.as_posix_thread().unwrap().tid());

        let entry = self.get_or_create_entry(tid);
        entry.set_thread(thread);
    }

    /// Removes a thread from the table.
    pub(super) fn remove_thread(&mut self, tid: Tid) {
        let Entry::Occupied(map_entry) = self.entries.entry(tid) else {
            return;
        };

        if map_entry.get().with_inner(|inner| {
            inner.clear_thread();
            inner.is_empty()
        }) {
            map_entry.remove();
        }
    }

    /// Removes a thread from the table and returns it.
    pub(super) fn take_thread(&mut self, tid: Tid) -> Option<Arc<Thread>> {
        let Entry::Occupied(map_entry) = self.entries.entry(tid) else {
            return None;
        };

        let (thread, is_empty) = map_entry.get().with_inner(|inner| {
            let thread = inner.thread.upgrade()?;
            inner.clear_thread();
            let is_empty = inner.is_empty();
            Some((thread, is_empty))
        })?;
        if is_empty {
            map_entry.remove();
        }

        Some(thread)
    }

    /// Replaces the live thread reference for a TID.
    pub(super) fn replace_thread(&mut self, tid: Tid, thread: &Arc<Thread>) {
        debug_assert_eq!(tid, thread.as_posix_thread().unwrap().tid());

        let entry = self.get_or_create_entry(tid);
        entry.replace_thread(thread);
    }

    /// Gets a thread by a TID.
    pub fn get_thread(&self, tid: Tid) -> Option<Arc<Thread>> {
        self.entries.get(&tid).and_then(|entry| entry.thread())
    }

    /// Returns an iterator over threads that have a live thread reference.
    pub fn iter_threads(&self) -> impl Iterator<Item = Arc<Thread>> + '_ {
        self.entries.values().filter_map(|entry| entry.thread())
    }

    // ---- Process operations ----

    /// Inserts a process into the table.
    pub(super) fn insert_process(&mut self, pid: Pid, process: &Arc<Process>) {
        debug_assert!(
            !self
                .entries
                .get(&pid)
                .is_some_and(|entry| entry.has_live_process())
        );
        self.process_count += 1;

        let entry = self.get_or_create_entry(pid);
        entry.set_process(process);
    }

    /// Removes a process from the table and notifies observers.
    pub(super) fn remove_process(&mut self, pid: Pid) {
        let Entry::Occupied(map_entry) = self.entries.entry(pid) else {
            return;
        };

        self.process_count -= 1;
        if map_entry.get().with_inner(|inner| {
            inner.clear_process();
            inner.is_empty()
        }) {
            map_entry.remove();
        }
    }

    /// Gets a process by a PID.
    pub fn get_process(&self, pid: Pid) -> Option<Arc<Process>> {
        self.entries.get(&pid).and_then(|entry| entry.process())
    }

    /// Returns an iterator over processes that have a live process reference.
    pub fn iter_processes(&self) -> impl Iterator<Item = Arc<Process>> + '_ {
        self.entries.values().filter_map(|entry| entry.process())
    }

    /// Returns an iterator over PID entries that still track a live process.
    pub fn iter_process_entries(&self) -> impl Iterator<Item = Arc<PidEntry>> + '_ {
        self.entries
            .values()
            .filter(|entry| entry.process().is_some())
            .cloned()
    }

    /// Returns the number of live processes.
    pub fn process_count(&self) -> usize {
        self.process_count
    }

    // ---- Process group operations ----

    /// Inserts a process group into the table.
    pub(super) fn insert_process_group(&mut self, pgid: Pgid, group: &Arc<ProcessGroup>) {
        let entry = self.get_or_create_entry(pgid);
        entry.set_process_group(group);
    }

    /// Removes a process group from the table.
    pub(super) fn remove_process_group(&mut self, pgid: Pgid) {
        let Entry::Occupied(map_entry) = self.entries.entry(pgid) else {
            return;
        };

        if map_entry.get().with_inner(|inner| {
            inner.clear_process_group();
            inner.is_empty()
        }) {
            map_entry.remove();
        }
    }

    /// Gets a process group by a PGID.
    pub fn get_process_group(&self, pgid: &Pgid) -> Option<Arc<ProcessGroup>> {
        self.entries
            .get(pgid)
            .and_then(|entry| entry.process_group())
    }

    /// Returns whether a process group with the given PGID exists.
    pub fn contains_process_group(&self, pgid: &Pgid) -> bool {
        self.entries
            .get(pgid)
            .is_some_and(|entry| entry.has_live_process_group())
    }

    // ---- Session operations ----

    /// Inserts a session into the table.
    pub(super) fn insert_session(&mut self, sid: Sid, session: &Arc<Session>) {
        let entry = self.get_or_create_entry(sid);
        entry.set_session(session);
    }

    /// Removes a session from the table.
    pub(super) fn remove_session(&mut self, sid: Sid) {
        let Entry::Occupied(map_entry) = self.entries.entry(sid) else {
            return;
        };

        if map_entry.get().with_inner(|inner| {
            inner.clear_session();
            inner.is_empty()
        }) {
            map_entry.remove();
        }
    }

    /// Returns the entry for the given numeric identifier.
    pub fn get_entry(&self, id: u32) -> Option<Arc<PidEntry>> {
        self.entries.get(&id).cloned()
    }
}

/// An entry in the unified PID table.
///
/// Each entry stores references to the thread, process, process group, and
/// session that share the same numeric identifier. Not all slots need to be
/// occupied at the same time.
///
/// These references are stored as `Weak` so the PID table remains a lookup
/// index rather than an owner, matching Linux's `struct pid`. This also avoids
/// future reference cycles once processes hold references to their
/// corresponding `PidEntry`s. With this ownership model, processes are owned
/// by their parents, while process groups and sessions are owned by their
/// member processes and are reclaimed automatically after the last process is reaped.
pub struct PidEntry {
    inner: Mutex<PidEntryInner>,
}

struct PidEntryInner {
    thread: Weak<Thread>,
    process: Weak<Process>,
    process_group: Weak<ProcessGroup>,
    session: Weak<Session>,
}

impl PidEntryInner {
    fn clear_thread(&mut self) {
        debug_assert!(!self.thread.is_dangling());
        self.thread = Weak::new();
    }

    fn clear_process(&mut self) {
        debug_assert!(!self.process.is_dangling());
        self.process = Weak::new();
    }

    fn clear_process_group(&mut self) {
        debug_assert!(!self.process_group.is_dangling());
        self.process_group = Weak::new();
    }

    fn clear_session(&mut self) {
        debug_assert!(!self.session.is_dangling());
        self.session = Weak::new();
    }

    fn is_empty(&self) -> bool {
        self.thread.is_dangling()
            && self.process.is_dangling()
            && self.process_group.is_dangling()
            && self.session.is_dangling()
    }
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

    fn with_inner<R>(&self, f: impl FnOnce(&mut PidEntryInner) -> R) -> R {
        let mut inner = self.inner.lock();
        f(&mut inner)
    }

    /// Sets the thread reference.
    fn set_thread(&self, thread: &Arc<Thread>) {
        let mut inner = self.inner.lock();
        debug_assert!(inner.thread.is_dangling());
        inner.thread = Arc::downgrade(thread);
    }

    /// Replaces the thread reference.
    fn replace_thread(&self, thread: &Arc<Thread>) {
        let mut inner = self.inner.lock();
        debug_assert!(!inner.thread.is_dangling());
        inner.thread = Arc::downgrade(thread);
    }

    /// Sets the process reference.
    fn set_process(&self, process: &Arc<Process>) {
        let mut inner = self.inner.lock();
        debug_assert!(inner.process.is_dangling());
        inner.process = Arc::downgrade(process);
    }

    /// Sets the process group reference.
    fn set_process_group(&self, group: &Arc<ProcessGroup>) {
        let mut inner = self.inner.lock();
        debug_assert!(inner.process_group.is_dangling());
        inner.process_group = Arc::downgrade(group);
    }

    /// Sets the session reference.
    fn set_session(&self, session: &Arc<Session>) {
        let mut inner = self.inner.lock();
        debug_assert!(inner.session.is_dangling());
        inner.session = Arc::downgrade(session);
    }

    /// Returns the thread associated with the entry, if any.
    pub fn thread(&self) -> Option<Arc<Thread>> {
        self.inner.lock().thread.upgrade()
    }

    /// Returns the process associated with the entry, if any.
    pub fn process(&self) -> Option<Arc<Process>> {
        self.inner.lock().process.upgrade()
    }

    /// Returns the process group associated with the entry, if any.
    fn process_group(&self) -> Option<Arc<ProcessGroup>> {
        self.inner.lock().process_group.upgrade()
    }

    /// Returns whether the entry still tracks a live process.
    fn has_live_process(&self) -> bool {
        self.inner.lock().process.upgrade().is_some()
    }

    /// Returns whether the entry still tracks a live process group.
    fn has_live_process_group(&self) -> bool {
        self.inner.lock().process_group.upgrade().is_some()
    }
}

/// Acquires a mutable reference to the global PID table.
pub fn pid_table_mut() -> MutexGuard<'static, PidTable> {
    PID_TABLE.lock()
}

/// Extension methods for `Weak<T>` values stored in `PidEntry`.
///
/// In this file, `Weak::new()` is used as a sentinel that represents an empty
/// slot. This trait provides a small helper for recognizing that state.
trait WeakIsDangling {
    /// Returns `true` if `self` is the empty-slot sentinel `Weak::new()`.
    fn is_dangling(&self) -> bool;
}

impl<T> WeakIsDangling for Weak<T> {
    fn is_dangling(&self) -> bool {
        Weak::ptr_eq(self, &Weak::new())
    }
}
