// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::vec_deque::VecDeque,
    sync::{Arc, Weak},
};
use core::sync::atomic::{AtomicBool, Ordering};

use keyable_arc::{KeyableArc, KeyableWeak};
use ostd::sync::{LocalIrqDisabled, Mutex, MutexGuard, SpinLock, SpinLockGuard};

use super::{EpollEvent, EpollFlags};
use crate::{
    events::{self, IoEvents},
    fs::{file_handle::FileLike, file_table::FileDesc},
    process::signal::{PollHandle, Pollee},
};

/// An epoll entry that is contained in an epoll file.
///
/// Each epoll entry can be added, modified, or deleted by the `EpollCtl` command.
pub(super) struct Entry {
    // The file descriptor and the file.
    key: EntryKey,
    // The event masks and flags.
    inner: Mutex<Inner>,
    // The observer that receives events.
    //
    // Keep this in a separate `Arc` to avoid dropping `Entry` in the observer callback, which may
    // cause deadlocks.
    observer: Arc<Observer>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct EntryKey {
    fd: FileDesc,
    file: KeyableWeak<dyn FileLike>,
}

impl From<(FileDesc, KeyableWeak<dyn FileLike>)> for EntryKey {
    fn from(value: (FileDesc, KeyableWeak<dyn FileLike>)) -> Self {
        Self {
            fd: value.0,
            file: value.1,
        }
    }
}

impl From<(FileDesc, &Arc<dyn FileLike>)> for EntryKey {
    fn from(value: (FileDesc, &Arc<dyn FileLike>)) -> Self {
        Self {
            fd: value.0,
            file: KeyableWeak::from(Arc::downgrade(value.1)),
        }
    }
}

struct Inner {
    event: EpollEvent,
    flags: EpollFlags,
    poller: PollHandle,
}

impl Entry {
    /// Creates a new epoll entry associated with the given epoll file.
    pub(super) fn new(
        fd: FileDesc,
        file: KeyableWeak<dyn FileLike>,
        ready_set: Arc<ReadySet>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| {
            let observer = Arc::new(Observer::new(ready_set, me.clone()));

            let inner = Inner {
                event: EpollEvent {
                    events: IoEvents::empty(),
                    user_data: 0,
                },
                flags: EpollFlags::empty(),
                poller: PollHandle::new(Arc::downgrade(&observer) as _),
            };

            Self {
                key: EntryKey { fd, file },
                inner: Mutex::new(inner),
                observer,
            }
        })
    }

    /// Gets the file associated with this epoll entry.
    ///
    /// Since an epoll entry only holds a weak reference to the file,
    /// it is possible (albeit unlikely) that the file has been dropped.
    fn file(&self) -> Option<Arc<dyn FileLike>> {
        self.key.file.upgrade().map(KeyableArc::into)
    }

    /// Polls the events of the file associated with this epoll entry.
    ///
    /// This method returns `None` if the file is dead. Otherwise, it returns the epoll event (if
    /// any) and a boolean value indicating whether the entry should be kept in the ready list
    /// (`true`) or removed from the ready list (`false`).
    pub(super) fn poll(&self) -> Option<(Option<EpollEvent>, bool)> {
        let file = self.file()?;
        let inner = self.inner.lock();

        // There are no events if the entry is disabled. Note that this check should be done after
        // locking `Inner` to avoid race conditions.
        if !self.observer.is_enabled() {
            return Some((None, false));
        }

        // Check whether the entry's file has some events.
        let io_events = file.poll(inner.event.events, None);

        // If this entry's file has some events, we need to return them.
        let ep_event = if !io_events.is_empty() {
            Some(EpollEvent::new(io_events, inner.event.user_data))
        } else {
            None
        };

        // If there are events and the epoll entry is neither edge-triggered nor one-shot, we need
        // to keep the entry in the ready list.
        let is_still_ready = ep_event.is_some()
            && !inner
                .flags
                .intersects(EpollFlags::EDGE_TRIGGER | EpollFlags::ONE_SHOT);

        // If there are events and the epoll entry is one-shot, we need to disable the entry until
        // the user enables it again via `EpollCtl::Mod`.
        if ep_event.is_some() && inner.flags.contains(EpollFlags::ONE_SHOT) {
            self.observer.reset_enabled(&inner);
        }

        Some((ep_event, is_still_ready))
    }

    /// Updates the epoll entry by the given event masks and flags.
    ///
    /// This method needs to be called in response to `EpollCtl::Add` and `EpollCtl::Mod`.
    pub(super) fn update(&self, event: EpollEvent, flags: EpollFlags) -> IoEvents {
        let file = self.file().unwrap();

        let mut inner = self.inner.lock();

        inner.event = event;
        inner.flags = flags;

        self.observer.set_enabled(&inner);

        file.poll(event.events, Some(&mut inner.poller))
    }

    /// Shuts down the epoll entry.
    ///
    /// This method needs to be called in response to `EpollCtl::Del`.
    pub(super) fn shutdown(&self) {
        let mut inner = self.inner.lock();

        self.observer.reset_enabled(&inner);
        inner.poller.reset();
    }

    /// Gets the underlying observer.
    pub(super) fn observer(&self) -> &Observer {
        &self.observer
    }

    /// Gets the key associated with the epoll entry.
    pub(super) fn key(&self) -> &EntryKey {
        &self.key
    }

    /// Gets the file descriptor associated with the epoll entry.
    pub(super) fn fd(&self) -> FileDesc {
        self.key.fd
    }

    /// Gets the file associated with this epoll entry.
    pub(super) fn file_weak(&self) -> &KeyableWeak<dyn FileLike> {
        &self.key.file
    }
}

/// A observer for [`Entry`] that can receive events.
pub(super) struct Observer {
    // Whether the entry is enabled.
    is_enabled: AtomicBool,
    // Whether the entry is in the ready list.
    is_ready: AtomicBool,
    // The ready set of the epoll file that contains this epoll entry.
    ready_set: Arc<ReadySet>,
    // The epoll entry itself (always inside an `Arc`).
    weak_entry: Weak<Entry>,
}

impl Observer {
    fn new(ready_set: Arc<ReadySet>, weak_entry: Weak<Entry>) -> Self {
        Self {
            is_enabled: AtomicBool::new(false),
            is_ready: AtomicBool::new(false),
            ready_set,
            weak_entry,
        }
    }

    /// Returns whether the epoll entry is in the ready list.
    ///
    /// This method needs to be called while holding the lock of the ready list. See also
    /// [`Self::set_ready`] and [`Self::reset_ready`].
    fn is_ready(&self, _guard: &SpinLockGuard<VecDeque<Weak<Entry>>, LocalIrqDisabled>) -> bool {
        self.is_ready.load(Ordering::Relaxed)
    }

    /// Marks the epoll entry as being in the ready list.
    ///
    /// This method must be called while holding the lock of the ready list. This is the only way
    /// to ensure that the "is ready" state matches the fact that the entry is actually in the
    /// ready list.
    fn set_ready(&self, _guard: &SpinLockGuard<VecDeque<Weak<Entry>>, LocalIrqDisabled>) {
        self.is_ready.store(true, Ordering::Relaxed);
    }

    /// Marks the epoll entry as not being in the ready list.
    ///
    /// This method must be called while holding the lock of the ready list. This is the only way
    /// to ensure that the "is ready" state matches the fact that the entry is actually in the
    /// ready list.
    fn reset_ready(&self, _guard: &SpinLockGuard<VecDeque<Weak<Entry>>, LocalIrqDisabled>) {
        self.is_ready.store(false, Ordering::Relaxed)
    }

    /// Returns whether the epoll entry is enabled.
    ///
    /// *Caution:* If this method is called without holding the lock of the event masks and flags,
    /// the user must ensure that the behavior is desired with respect to the way the event masks
    /// and flags might be modified concurrently.
    fn is_enabled(&self) -> bool {
        self.is_enabled.load(Ordering::Relaxed)
    }

    /// Marks the epoll entry as enabled.
    ///
    /// This method must be called while holding the lock of the event masks and flags. This is the
    /// only way to ensure that the "is enabled" state describes the correct combination of the
    /// event masks and flags.
    fn set_enabled(&self, _guard: &MutexGuard<Inner>) {
        self.is_enabled.store(true, Ordering::Relaxed)
    }

    /// Marks the epoll entry as not enabled.
    ///
    /// This method must be called while holding the lock of the event masks and flags. This is the
    /// only way to ensure that the "is enabled" state describes the correct combination of the
    /// event masks and flags.
    fn reset_enabled(&self, _guard: &MutexGuard<Inner>) {
        self.is_enabled.store(false, Ordering::Relaxed)
    }

    /// Gets an instance of `Weak` that refers to the epoll entry.
    fn weak_entry(&self) -> &Weak<Entry> {
        &self.weak_entry
    }
}

impl events::Observer<IoEvents> for Observer {
    fn on_events(&self, _events: &IoEvents) {
        self.ready_set.push(self);
    }
}

/// A set of ready epoll entries.
pub(super) struct ReadySet {
    // Entries that are probably ready (having events happened).
    entries: SpinLock<VecDeque<Weak<Entry>>, LocalIrqDisabled>,
    // A guard to ensure that ready entries can be popped by one thread at a time.
    pop_guard: Mutex<PopGuard>,
    // A pollee for the ready set (i.e., for `EpollFile` itself).
    pollee: Pollee,
}

struct PopGuard;

impl ReadySet {
    pub(super) fn new() -> Self {
        Self {
            entries: SpinLock::new(VecDeque::new()),
            pop_guard: Mutex::new(PopGuard),
            pollee: Pollee::new(),
        }
    }

    pub(super) fn push(&self, observer: &Observer) {
        // Note that we cannot take the `Inner` lock because we may be in the callback of the event
        // observer. Doing so will cause dead locks due to inconsistent locking orders.
        //
        // We don't need to take the lock because
        // - `set_enabled` is guaranteed to be visible after its following `FileLike::poll`. This
        //   synchronization relies on the lock in `Subject`. As a result, no events are lost.
        // - Catching spurious events here is always fine because we always check them later before
        //   returning events to the user (in `Entry::poll`).
        if !observer.is_enabled() {
            return;
        }

        let mut entries = self.entries.lock();

        if !observer.is_ready(&entries) {
            observer.set_ready(&entries);
            entries.push_back(observer.weak_entry().clone())
        }

        // Even if the entry is already set to ready,
        // there might be new events that we are interested in.
        // Wake the poller anyway.
        self.pollee.notify(IoEvents::IN);
    }

    pub(super) fn lock_pop(&self) -> ReadySetPopIter {
        ReadySetPopIter {
            ready_set: self,
            _pop_guard: self.pop_guard.lock(),
            limit: None,
        }
    }

    pub(super) fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }

    fn check_io_events(&self) -> IoEvents {
        let entries = self.entries.lock();

        if !entries.is_empty() {
            IoEvents::IN
        } else {
            IoEvents::empty()
        }
    }
}

/// An iterator to pop ready entries from a [`ReadySet`].
pub(super) struct ReadySetPopIter<'a> {
    ready_set: &'a ReadySet,
    _pop_guard: MutexGuard<'a, PopGuard>,
    limit: Option<usize>,
}

impl Iterator for ReadySetPopIter<'_> {
    type Item = Arc<Entry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.limit == Some(0) {
            return None;
        }

        let mut entries = self.ready_set.entries.lock();
        let mut limit = self.limit.unwrap_or_else(|| entries.len());

        while limit > 0 {
            limit -= 1;

            // Pop the front entry. Note that `_pop_guard` and `limit` guarantee that this entry
            // must exist, so we can just unwrap it.
            let weak_entry = entries.pop_front().unwrap();

            // Clear the epoll file's events if there are no ready entries.
            if entries.len() == 0 {
                self.ready_set.pollee.invalidate();
            }

            let Some(entry) = Weak::upgrade(&weak_entry) else {
                // The entry has been deleted.
                continue;
            };

            // Mark the entry as not ready. We can invoke `ReadySet::push` later to add it back to
            // the ready list if we need to.
            entry.observer().reset_ready(&entries);

            self.limit = Some(limit);
            return Some(entry);
        }

        self.limit = None;
        None
    }
}
