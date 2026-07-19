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

    /// Inserts a non-main thread into the table.
    ///
    /// This method requires the target entry not to track a process. A
    /// process's main thread must be inserted with [`Self::insert_process`].
    pub(super) fn insert_thread(&mut self, tid: Tid, thread: &Arc<Thread>) {
        debug_assert_eq!(tid, thread.as_posix_thread().unwrap().tid());

        let mut entry = self.get_or_create_entry(tid).lock();
        debug_assert!(!entry.has_live_process());

        entry.set_thread(thread);
    }

    /// Removes a non-main thread from the table.
    ///
    /// This method requires the target entry not to track a process. A
    /// process's main thread must be removed with [`Self::remove_process`].
    pub(super) fn remove_thread(&mut self, tid: Tid) {
        let Entry::Occupied(map_entry) = self.entries.entry(tid) else {
            return;
        };

        let should_remove = {
            let mut pid_entry = map_entry.get().lock();
            debug_assert!(!pid_entry.has_live_process());

            pid_entry.clear_thread();
            // Drop the locked PID entry before removing the B-tree entry.
            pid_entry.is_empty()
        };

        if should_remove {
            map_entry.remove();
        }
    }

    /// Removes a non-main thread from the table and returns it.
    ///
    /// This method requires the target entry not to track a process.
    pub(super) fn take_thread(&mut self, tid: Tid) -> Option<Arc<Thread>> {
        let Entry::Occupied(map_entry) = self.entries.entry(tid) else {
            return None;
        };

        let (thread, should_remove) = {
            let mut pid_entry = map_entry.get().lock();
            debug_assert!(!pid_entry.has_live_process());

            let thread = pid_entry.thread()?;
            pid_entry.clear_thread();
            // Drop the locked PID entry before removing the B-tree entry.
            (thread, pid_entry.is_empty())
        };

        if should_remove {
            map_entry.remove();
        }

        Some(thread)
    }

    /// Replaces the live thread reference for a TID.
    pub(super) fn replace_thread(&mut self, tid: Tid, thread: &Arc<Thread>) {
        debug_assert_eq!(tid, thread.as_posix_thread().unwrap().tid());

        let entry = self.get_or_create_entry(tid);
        entry.lock().replace_thread(thread);
    }

    /// Gets a thread by a TID.
    pub fn get_thread(&self, tid: Tid) -> Option<Arc<Thread>> {
        self.entries
            .get(&tid)
            .and_then(|entry| entry.lock().thread())
    }

    /// Returns an iterator over threads that have a live thread reference.
    pub fn iter_threads(&self) -> impl Iterator<Item = Arc<Thread>> + '_ {
        self.entries
            .values()
            .filter_map(|entry| entry.lock().thread())
    }

    // ---- Process operations ----

    /// Inserts a process and its main thread into the table.
    pub(super) fn insert_process(&mut self, pid: Pid, process: &Arc<Process>) {
        // `set_process` will assert the process slot is empty.
        self.process_count += 1;

        let entry = self.get_or_create_entry(pid);
        let mut entry = entry.lock();
        entry.set_process(process);
        entry.set_thread(&process.main_thread());
    }

    /// Removes a process and its main thread from the table.
    //
    // TODO: Add an active reclamation mechanism for dentries corresponding to `PidEntry`
    // in the procfs `DentryCache`, so that invalid dentries can be released as promptly
    // as possible.
    pub(super) fn remove_process(&mut self, pid: Pid) {
        let Entry::Occupied(map_entry) = self.entries.entry(pid) else {
            return;
        };

        let should_remove = {
            let mut pid_entry = map_entry.get().lock();

            // `clear_process` will assert the process slot is not empty.
            self.process_count -= 1;

            pid_entry.clear_process();
            pid_entry.clear_thread();
            // Drop the locked PID entry before removing the B-tree entry.
            pid_entry.is_empty()
        };

        if should_remove {
            map_entry.remove();
        }
    }

    /// Gets a process by a PID.
    pub fn get_process(&self, pid: Pid) -> Option<Arc<Process>> {
        self.entries
            .get(&pid)
            .and_then(|entry| entry.lock().process())
    }

    /// Returns an iterator over processes that have a live process reference.
    pub fn iter_processes(&self) -> impl Iterator<Item = Arc<Process>> + '_ {
        self.entries
            .values()
            .filter_map(|entry| entry.lock().process())
    }

    /// Returns the number of live processes.
    pub fn process_count(&self) -> usize {
        self.process_count
    }

    // ---- Process group operations ----

    /// Inserts a process group into the table.
    pub(super) fn insert_process_group(&mut self, pgid: Pgid, group: &Arc<ProcessGroup>) {
        let entry = self.get_or_create_entry(pgid);
        entry.lock().set_process_group(group);
    }

    /// Removes a process group from the table.
    pub(super) fn remove_process_group(&mut self, pgid: Pgid) {
        let Entry::Occupied(map_entry) = self.entries.entry(pgid) else {
            return;
        };

        let should_remove = {
            let mut pid_entry = map_entry.get().lock();
            pid_entry.clear_process_group();
            // Drop the locked PID entry before removing the B-tree entry.
            pid_entry.is_empty()
        };

        if should_remove {
            map_entry.remove();
        }
    }

    /// Gets a process group by a PGID.
    pub fn get_process_group(&self, pgid: &Pgid) -> Option<Arc<ProcessGroup>> {
        self.entries
            .get(pgid)
            .and_then(|entry| entry.lock().process_group())
    }

    /// Returns whether a process group with the given PGID exists.
    pub fn contains_process_group(&self, pgid: &Pgid) -> bool {
        self.entries
            .get(pgid)
            .is_some_and(|entry| entry.lock().has_live_process_group())
    }

    // ---- Session operations ----

    /// Inserts a session into the table.
    pub(super) fn insert_session(&mut self, sid: Sid, session: &Arc<Session>) {
        let entry = self.get_or_create_entry(sid);
        entry.lock().set_session(session);
    }

    /// Removes a session from the table.
    pub(super) fn remove_session(&mut self, sid: Sid) {
        let Entry::Occupied(map_entry) = self.entries.entry(sid) else {
            return;
        };

        let should_remove = {
            let mut pid_entry = map_entry.get().lock();
            pid_entry.clear_session();
            // Drop the locked PID entry before removing the B-tree entry.
            pid_entry.is_empty()
        };

        if should_remove {
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
///
/// # Atomicity of process/thread updates
///
/// [`PidTable`] guarantees that process/thread insertion and removal operations
/// are atomic with respect to the corresponding `PidEntry`. In other words,
/// there will never be a `PidEntry` in the [`PidTable`] that is associated with
/// a [`Process`], but at some intermediate moment has only an associated [`Thread`].
pub struct PidEntry {
    inner: Mutex<PidEntryInner>,
}

struct PidEntryInner {
    thread: Weak<Thread>,
    process: Weak<Process>,
    process_group: Weak<ProcessGroup>,
    session: Weak<Session>,
}

/// The process/thread type represented by a [`PidEntry`].
///
/// [`PidTable`]'s guarantees for process/thread update operations ensure that its
/// entry is never seen in an intermediate [`PidEntryType`].
pub enum PidEntryType {
    /// The entry tracks a live process. The associated thread, if any, is
    /// the process's main thread.
    Process,
    /// The entry tracks a non-main POSIX thread (one whose TID differs
    /// from any live process's PID).
    Thread,
}

impl PidEntry {
    /// Creates a new empty `PidEntry`.
    fn new() -> Self {
        Self {
            inner: Mutex::new(PidEntryInner::new()),
        }
    }

    /// Locks and returns access to the entry internals.
    fn lock(&self) -> MutexGuard<'_, PidEntryInner> {
        self.inner.lock()
    }

    /// Returns the thread associated with the entry, if any.
    pub fn thread(&self) -> Option<Arc<Thread>> {
        self.lock().thread()
    }

    /// Returns the process of the thread associated with the entry, if any.
    ///
    /// This method is not limited to the process slot.
    /// If the entry only tracks a thread,
    /// this returns the process that the thread belongs to.
    ///
    /// This is useful for procfs lookups that need process-scoped state for
    /// either `/proc/[pid]` or `/proc/[pid]/task/[tid]`.
    pub fn process_of_thread(&self) -> Option<Arc<Process>> {
        let inner = self.lock();

        if let Some(process) = inner.process() {
            return Some(process);
        }

        if let Some(thread) = inner.thread() {
            return Some(thread.as_posix_thread().unwrap().process());
        }

        None
    }

    /// Returns whether the entry is associated with a process or a thread.
    ///
    /// If a process and a thread share this numeric ID, returns
    /// [`PidEntryType::Process`].
    pub fn type_(&self) -> Option<PidEntryType> {
        let inner = self.lock();

        if inner.has_live_process() {
            return Some(PidEntryType::Process);
        }

        if inner.has_live_thread() {
            return Some(PidEntryType::Thread);
        }

        None
    }
}

impl PidEntryInner {
    /// Creates a new empty `PidEntryInner`.
    fn new() -> Self {
        Self {
            thread: Weak::new(),
            process: Weak::new(),
            process_group: Weak::new(),
            session: Weak::new(),
        }
    }

    /// Sets the thread reference.
    fn set_thread(&mut self, thread: &Arc<Thread>) {
        debug_assert!(!self.has_live_thread());
        self.thread = Arc::downgrade(thread);
    }

    /// Clears the thread reference.
    fn clear_thread(&mut self) {
        debug_assert!(self.has_live_thread());
        self.thread = Weak::new();
    }

    /// Replaces the thread reference.
    fn replace_thread(&mut self, thread: &Arc<Thread>) {
        debug_assert!(self.has_live_thread());
        self.thread = Arc::downgrade(thread);
    }

    /// Sets the process reference.
    fn set_process(&mut self, process: &Arc<Process>) {
        debug_assert!(!self.has_live_process());
        self.process = Arc::downgrade(process);
    }

    /// Clears the process reference.
    fn clear_process(&mut self) {
        debug_assert!(self.has_live_process());
        self.process = Weak::new();
    }

    /// Sets the process group reference.
    fn set_process_group(&mut self, group: &Arc<ProcessGroup>) {
        debug_assert!(!self.has_live_process_group());
        self.process_group = Arc::downgrade(group);
    }

    /// Clears the process group reference.
    fn clear_process_group(&mut self) {
        debug_assert!(self.has_live_process_group());
        self.process_group = Weak::new();
    }

    /// Sets the session reference.
    fn set_session(&mut self, session: &Arc<Session>) {
        debug_assert!(!self.has_live_session());
        self.session = Arc::downgrade(session);
    }

    /// Clears the session reference.
    fn clear_session(&mut self) {
        debug_assert!(self.has_live_session());
        self.session = Weak::new();
    }

    /// Returns the thread associated with the entry, if any.
    fn thread(&self) -> Option<Arc<Thread>> {
        self.thread.upgrade()
    }

    /// Returns the process associated with the entry, if any.
    fn process(&self) -> Option<Arc<Process>> {
        self.process.upgrade()
    }

    /// Returns the process group associated with the entry, if any.
    fn process_group(&self) -> Option<Arc<ProcessGroup>> {
        self.process_group.upgrade()
    }

    /// Returns whether the entry still tracks a live thread.
    fn has_live_thread(&self) -> bool {
        !self.thread.is_dangling()
    }

    /// Returns whether the entry still tracks a live process.
    fn has_live_process(&self) -> bool {
        !self.process.is_dangling()
    }

    /// Returns whether the entry still tracks a live process group.
    fn has_live_process_group(&self) -> bool {
        !self.process_group.is_dangling()
    }

    /// Returns whether the entry still tracks a live session.
    fn has_live_session(&self) -> bool {
        !self.session.is_dangling()
    }

    /// Returns `true` if the entry no longer tracks any live object.
    fn is_empty(&self) -> bool {
        !self.has_live_thread()
            && !self.has_live_process()
            && !self.has_live_process_group()
            && !self.has_live_session()
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
