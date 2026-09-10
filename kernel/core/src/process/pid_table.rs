// SPDX-License-Identifier: MPL-2.0

//! A unified PID table that maps numeric identifiers to threads, processes,
//! process groups, and sessions.
//!
//! This design is inspired by Linux's `struct pid`. Each [`PidEntry`] tracks
//! the kernel objects that share the same numeric identifier, which eliminates
//! the need for separate per-type lookup tables.

use alloc::collections::btree_map::Entry;
use core::sync::atomic::{AtomicU32, Ordering};

use sparse_id_alloc::CyclicIdAlloc;

use super::{INIT_PROCESS_PID, Pgid, Pid, Process, ProcessGroup, Session, Sid};
use crate::{
    prelude::*,
    process::posix_thread::AsPosixThread,
    thread::{Thread, Tid},
};

static PID_TABLE: Mutex<PidTable> = Mutex::new(PidTable::new());

/// The PID allocation wrap value.
///
/// This matches Linux's `PID_MAX_LIMIT` on 64-bit architectures.
/// Reference: <https://elixir.bootlin.com/linux/v6.16/source/include/linux/threads.h#L34>.
// FIXME: This value cannot yet be modified by the user by writing to
// `/proc/sys/kernel/pid_max`.
pub(crate) const PID_MAX: u32 = 4 * 1024 * 1024;

/// The lowest TID that may be reused after the allocator wraps.
///
/// Linux reserves the lower 300 PIDs after the initial allocation pass.
/// Reference: <https://elixir.bootlin.com/linux/v6.16/source/include/linux/pid.h#L48>.
const PID_RECYCLE_MIN: Tid = 300;

static LAST_ALLOCATED_TID: AtomicU32 = AtomicU32::new(0);

/// The unified PID table.
///
/// Combines the process, process-group, session, and thread tables into a
/// single structure.
pub(crate) struct PidTable {
    entries: BTreeMap<u32, Arc<PidEntry>>,
    tid_allocator: CyclicIdAlloc,
    process_count: usize,
}

impl PidTable {
    const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            tid_allocator: CyclicIdAlloc::new(INIT_PROCESS_PID, PID_RECYCLE_MIN, PID_MAX - 1),
            process_count: 0,
        }
    }

    /// Returns the allocated entry for an associated process group or session.
    ///
    /// ID zero is used by the bootstrap process group and session but is never
    /// handed out by the TID allocator.
    fn associated_entry(&mut self, id: u32) -> &Arc<PidEntry> {
        if id == 0 {
            return self
                .entries
                .entry(id)
                .or_insert_with(|| Arc::new(PidEntry::new(id, false)));
        }

        let entry = self
            .entries
            .get(&id)
            .expect("a nonzero associated ID must already be allocated");
        debug_assert!(!entry.lock().is_reserved());
        entry
    }

    /// Reserves a new TID and creates an invisible [`PidEntry`] for it.
    fn reserve_tid(&mut self) -> Result<PidReservation> {
        let tid = self
            .tid_allocator
            .alloc()
            .ok_or_else(|| Error::with_message(Errno::EAGAIN, "the PID space is exhausted"))?;
        let entry = Arc::new(PidEntry::new(tid, true));
        let old_entry = self.entries.insert(tid, entry.clone());
        debug_assert!(old_entry.is_none());
        LAST_ALLOCATED_TID.store(tid, Ordering::Relaxed);

        Ok(PidReservation {
            tid,
            entry,
            active: true,
        })
    }

    /// Commits a reserved TID as a non-main thread.
    fn commit_thread(&mut self, reservation: &PidReservation, thread: &Arc<Thread>) {
        debug_assert_eq!(reservation.tid, thread.as_posix_thread().unwrap().tid());
        let entry = self.entries.get(&reservation.tid).unwrap();
        debug_assert!(Arc::ptr_eq(entry, &reservation.entry));

        let mut entry = entry.lock();
        entry.commit_reservation();
        debug_assert!(!entry.has_live_process());
        entry.set_thread(thread);
    }

    /// Commits a reserved TID as a process and its main thread.
    fn commit_process(&mut self, reservation: &PidReservation, process: &Arc<Process>) {
        debug_assert_eq!(reservation.tid, process.pid());
        let entry = self.entries.get(&reservation.tid).unwrap();
        debug_assert!(Arc::ptr_eq(entry, &reservation.entry));

        self.process_count += 1;
        let mut entry = entry.lock();
        entry.commit_reservation();
        entry.set_process(process);
        entry.set_thread(&process.main_thread());
    }

    /// Cancels a reservation that was not committed.
    fn cancel_reservation(&mut self, reservation: &PidReservation) {
        let entry = self.entries.get(&reservation.tid).unwrap();
        debug_assert!(Arc::ptr_eq(entry, &reservation.entry));
        debug_assert!(entry.lock().is_reserved());

        self.entries.remove(&reservation.tid);
        self.tid_allocator.free(reservation.tid);
    }

    /// Removes an empty PID entry and returns its number to the allocator.
    fn remove_entry_if_empty(&mut self, id: Tid) {
        let should_remove = self
            .entries
            .get(&id)
            .is_some_and(|entry| entry.lock().is_empty());
        if should_remove {
            self.entries.remove(&id);
            if id != 0 {
                self.tid_allocator.free(id);
            }
        }
    }

    // ---- Thread operations ----

    /// Removes a non-main thread from the table.
    ///
    /// This method requires the target entry not to track a process. A
    /// process's main thread must be removed with [`Self::remove_process`].
    pub(super) fn remove_thread(&mut self, tid: Tid) {
        {
            let Entry::Occupied(map_entry) = self.entries.entry(tid) else {
                return;
            };

            let mut pid_entry = map_entry.get().lock();
            debug_assert!(!pid_entry.has_live_process());

            pid_entry.clear_thread();
        }
        self.remove_entry_if_empty(tid);
    }

    /// Removes a non-main thread from the table and returns it.
    ///
    /// This method requires the target entry not to track a process.
    pub(super) fn take_thread(&mut self, tid: Tid) -> Option<Arc<Thread>> {
        let thread = {
            let Entry::Occupied(map_entry) = self.entries.entry(tid) else {
                return None;
            };

            let mut pid_entry = map_entry.get().lock();
            debug_assert!(!pid_entry.has_live_process());

            let thread = pid_entry.thread()?;
            pid_entry.clear_thread();
            thread
        };

        self.remove_entry_if_empty(tid);

        Some(thread)
    }

    /// Replaces the live thread reference for a TID.
    pub(super) fn replace_thread(&mut self, tid: Tid, thread: &Arc<Thread>) {
        debug_assert_eq!(tid, thread.as_posix_thread().unwrap().tid());

        let entry = self.entries.get(&tid).unwrap();
        entry.lock().replace_thread(thread);
    }

    /// Gets a thread by a TID.
    pub(crate) fn get_thread(&self, tid: Tid) -> Option<Arc<Thread>> {
        self.entries
            .get(&tid)
            .and_then(|entry| entry.lock().thread())
    }

    /// Returns an iterator over threads that have a live thread reference.
    pub(crate) fn iter_threads(&self) -> impl Iterator<Item = Arc<Thread>> + '_ {
        self.entries
            .values()
            .filter_map(|entry| entry.lock().thread())
    }

    // ---- Process operations ----

    /// Removes a process and its main thread from the table.
    //
    // TODO: Add an active reclamation mechanism for dentries corresponding to `PidEntry`
    // in the procfs `DentryCache`, so that invalid dentries can be released as promptly
    // as possible.
    pub(super) fn remove_process(&mut self, pid: Pid) {
        {
            let Entry::Occupied(map_entry) = self.entries.entry(pid) else {
                return;
            };

            let mut pid_entry = map_entry.get().lock();

            // `clear_process` will assert the process slot is not empty.
            self.process_count -= 1;

            pid_entry.clear_process();
            pid_entry.clear_thread();
        }
        self.remove_entry_if_empty(pid);
    }

    /// Gets a process by a PID.
    pub(crate) fn get_process(&self, pid: Pid) -> Option<Arc<Process>> {
        self.entries
            .get(&pid)
            .and_then(|entry| entry.lock().process())
    }

    /// Returns an iterator over processes that have a live process reference.
    pub(crate) fn iter_processes(&self) -> impl Iterator<Item = Arc<Process>> + '_ {
        self.entries
            .values()
            .filter_map(|entry| entry.lock().process())
    }

    /// Returns the number of live processes.
    pub(crate) fn process_count(&self) -> usize {
        self.process_count
    }

    // ---- Process group operations ----

    /// Inserts a process group into the table.
    pub(super) fn insert_process_group(&mut self, pgid: Pgid, group: &Arc<ProcessGroup>) {
        let entry = self.associated_entry(pgid);
        entry.lock().set_process_group(group);
    }

    /// Removes a process group from the table.
    pub(super) fn remove_process_group(&mut self, pgid: Pgid) {
        {
            let Entry::Occupied(map_entry) = self.entries.entry(pgid) else {
                return;
            };

            let mut pid_entry = map_entry.get().lock();
            pid_entry.clear_process_group();
        }
        self.remove_entry_if_empty(pgid);
    }

    /// Gets a process group by a PGID.
    pub(crate) fn get_process_group(&self, pgid: &Pgid) -> Option<Arc<ProcessGroup>> {
        self.entries
            .get(pgid)
            .and_then(|entry| entry.lock().process_group())
    }

    /// Returns whether a process group with the given PGID exists.
    pub(crate) fn contains_process_group(&self, pgid: &Pgid) -> bool {
        self.entries
            .get(pgid)
            .is_some_and(|entry| entry.lock().has_live_process_group())
    }

    // ---- Session operations ----

    /// Inserts a session into the table.
    pub(super) fn insert_session(&mut self, sid: Sid, session: &Arc<Session>) {
        let entry = self.associated_entry(sid);
        entry.lock().set_session(session);
    }

    /// Removes a session from the table.
    pub(super) fn remove_session(&mut self, sid: Sid) {
        {
            let Entry::Occupied(map_entry) = self.entries.entry(sid) else {
                return;
            };

            let mut pid_entry = map_entry.get().lock();
            pid_entry.clear_session();
        }
        self.remove_entry_if_empty(sid);
    }

    /// Returns the visible entry for the given numeric identifier.
    pub(crate) fn get_entry(&self, id: u32) -> Option<Arc<PidEntry>> {
        self.entries
            .get(&id)
            .filter(|entry| !entry.lock().is_reserved())
            .cloned()
    }
}

/// A TID reserved for a process or thread that is still being constructed.
///
/// The reservation is invisible to PID lookups. Dropping it before it is
/// committed returns the number to the allocator.
pub(super) struct PidReservation {
    tid: Tid,
    entry: Arc<PidEntry>,
    active: bool,
}

impl PidReservation {
    /// Returns the reserved TID.
    pub(super) fn tid(&self) -> Tid {
        self.tid
    }

    /// Returns the stable PID entry associated with the reservation.
    pub(super) fn pid_entry(&self) -> Arc<PidEntry> {
        self.entry.clone()
    }

    /// Commits the reservation as a non-main thread while the caller holds the PID table.
    pub(super) fn commit_thread(mut self, pid_table: &mut PidTable, thread: &Arc<Thread>) {
        pid_table.commit_thread(&self, thread);
        self.active = false;
    }

    /// Commits the reservation as a process while the caller holds the PID table.
    pub(super) fn commit_process(mut self, pid_table: &mut PidTable, process: &Arc<Process>) {
        pid_table.commit_process(&self, process);
        self.active = false;
    }
}

impl Drop for PidReservation {
    fn drop(&mut self) {
        if self.active {
            pid_table_mut().cancel_reservation(self);
        }
    }
}

/// Reserves a new TID from the global PID table.
pub(super) fn reserve_tid() -> Result<PidReservation> {
    pid_table_mut().reserve_tid()
}

/// Returns the most recently reserved TID.
pub(crate) fn last_tid() -> Tid {
    LAST_ALLOCATED_TID.load(Ordering::Relaxed)
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
pub(crate) struct PidEntry {
    id: Tid,
    inner: Mutex<PidEntryInner>,
}

struct PidEntryInner {
    reserved: bool,
    thread: Weak<Thread>,
    process: Weak<Process>,
    process_group: Weak<ProcessGroup>,
    session: Weak<Session>,
}

/// The process/thread type represented by a [`PidEntry`].
///
/// [`PidTable`]'s guarantees for process/thread update operations ensure that its
/// entry is never seen in an intermediate [`PidEntryType`].
pub(crate) enum PidEntryType {
    /// The entry tracks a live process. The associated thread, if any, is
    /// the process's main thread.
    Process,
    /// The entry tracks a non-main POSIX thread (one whose TID differs
    /// from any live process's PID).
    Thread,
}

impl PidEntry {
    /// Creates a new PID entry.
    fn new(id: Tid, reserved: bool) -> Self {
        Self {
            id,
            inner: Mutex::new(PidEntryInner::new(reserved)),
        }
    }

    /// Returns the numeric identifier represented by this entry.
    pub(super) fn id(&self) -> Tid {
        self.id
    }

    /// Locks and returns access to the entry internals.
    fn lock(&self) -> MutexGuard<'_, PidEntryInner> {
        self.inner.lock()
    }

    /// Returns the thread associated with the entry, if any.
    pub(crate) fn thread(&self) -> Option<Arc<Thread>> {
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
    pub(crate) fn process_of_thread(&self) -> Option<Arc<Process>> {
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
    pub(crate) fn type_(&self) -> Option<PidEntryType> {
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
    fn new(reserved: bool) -> Self {
        Self {
            reserved,
            thread: Weak::new(),
            process: Weak::new(),
            process_group: Weak::new(),
            session: Weak::new(),
        }
    }

    /// Marks a reserved entry as ready to expose its associated object.
    fn commit_reservation(&mut self) {
        debug_assert!(self.reserved);
        self.reserved = false;
    }

    /// Returns whether this entry is reserved for an object under construction.
    fn is_reserved(&self) -> bool {
        self.reserved
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
        !self.reserved
            && !self.has_live_thread()
            && !self.has_live_process()
            && !self.has_live_process_group()
            && !self.has_live_session()
    }
}

/// Acquires a mutable reference to the global PID table.
pub(crate) fn pid_table_mut() -> MutexGuard<'static, PidTable> {
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
