// SPDX-License-Identifier: MPL-2.0

use core::{
    borrow::Borrow,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use keyable_arc::{KeyableArc, KeyableWeak};
use ostd::sync::LocalIrqDisabled;

use super::*;
use crate::{
    events::Observer,
    fs::{file_handle::FileLike, utils::IoctlCmd},
    process::signal::{Pollable, Pollee, Poller},
};

/// A file-like object that provides epoll API.
///
/// Conceptually, we maintain two lists: one consists of all interesting files,
/// which can be managed by the epoll ctl commands; the other are for ready files,
/// which are files that have some events. A epoll wait only needs to iterate the
/// ready list and poll each file to see if the file is ready for the interesting
/// I/O.
///
/// To maintain the ready list, we need to monitor interesting events that happen
/// on the files. To do so, the `EpollFile` registers itself as an `Observer` to
/// the monotored files. Thus, we can add a file to the ready list when an interesting
/// event happens on the file.
pub struct EpollFile {
    // All interesting entries.
    interest: Mutex<BTreeSet<EpollEntryHolder>>,
    // Entries that are probably ready (having events happened).
    ready: SpinLock<VecDeque<Weak<EpollEntry>>, LocalIrqDisabled>,
    // A guard to ensure that ready entries can be popped by one thread at a time.
    pop_guard: Mutex<PopGuard>,
    // EpollFile itself is also pollable
    pollee: Pollee,
    // Any EpollFile is wrapped with Arc when created.
    weak_self: Weak<Self>,
}

struct PopGuard;

impl EpollFile {
    /// Creates a new epoll file.
    pub fn new() -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            interest: Mutex::new(BTreeSet::new()),
            ready: SpinLock::new(VecDeque::new()),
            pop_guard: Mutex::new(PopGuard),
            pollee: Pollee::new(IoEvents::empty()),
            weak_self: me.clone(),
        })
    }

    /// Control the interest list of the epoll file.
    pub fn control(&self, cmd: &EpollCtl) -> Result<()> {
        let fd = match cmd {
            EpollCtl::Add(fd, ..) => *fd,
            EpollCtl::Del(fd) => *fd,
            EpollCtl::Mod(fd, ..) => *fd,
        };

        let file = {
            let current = current!();
            current
                .file_table()
                .lock_with(|file_table| Result::Ok(file_table.get_file(fd)?.clone()))?
        };

        match *cmd {
            EpollCtl::Add(fd, ep_event, ep_flags) => {
                self.add_interest(fd, file, ep_event, ep_flags)
            }
            EpollCtl::Del(fd) => self.del_interest(fd, Arc::downgrade(&file).into()),
            EpollCtl::Mod(fd, ep_event, ep_flags) => {
                self.mod_interest(fd, file, ep_event, ep_flags)
            }
        }
    }

    fn add_interest(
        &self,
        fd: FileDesc,
        file: Arc<dyn FileLike>,
        ep_event: EpollEvent,
        ep_flags: EpollFlags,
    ) -> Result<()> {
        self.warn_unsupported_flags(&ep_flags);

        // Add the new entry to the interest list and start monitoring its events
        let ready_entry = {
            let mut interest = self.interest.lock();

            if interest.contains(&EpollEntryKey::from((fd, &file))) {
                return_errno_with_message!(
                    Errno::EEXIST,
                    "the file is already in the interest list"
                );
            }

            let entry = EpollEntry::new(fd, Arc::downgrade(&file).into(), self.weak_self.clone());
            let events = entry.update(ep_event, ep_flags)?;

            let ready_entry = if !events.is_empty() {
                Some(entry.clone())
            } else {
                None
            };

            let inserted = interest.insert(entry.into());
            assert!(inserted);

            ready_entry
        };

        // Add the new entry to the ready list if the file is ready
        if let Some(entry) = ready_entry {
            self.push_ready(entry);
        }

        Ok(())
    }

    fn del_interest(&self, fd: FileDesc, file: KeyableWeak<dyn FileLike>) -> Result<()> {
        // If this epoll entry is in the ready list, then we should delete it.
        // But unfortunately, deleting an entry from the ready list has a
        // complexity of O(N).
        //
        // To optimize performance, we postpone the actual deletion to the time
        // when the ready list is scanned in `EpolFile::wait`. This can be done
        // because the strong reference count will reach zero and `Weak::upgrade`
        // will fail.

        if !self
            .interest
            .lock()
            .remove(&EpollEntryKey::from((fd, file)))
        {
            return_errno_with_message!(Errno::ENOENT, "the file is not in the interest list");
        }

        Ok(())
    }

    fn mod_interest(
        &self,
        fd: FileDesc,
        file: Arc<dyn FileLike>,
        new_ep_event: EpollEvent,
        new_ep_flags: EpollFlags,
    ) -> Result<()> {
        self.warn_unsupported_flags(&new_ep_flags);

        // Update the epoll entry
        let ready_entry = {
            let interest = self.interest.lock();

            let EpollEntryHolder(entry) = interest
                .get(&EpollEntryKey::from((fd, &file)))
                .ok_or_else(|| {
                    Error::with_message(Errno::ENOENT, "the file is not in the interest list")
                })?;
            let events = entry.update(new_ep_event, new_ep_flags)?;

            if !events.is_empty() {
                Some(entry.clone())
            } else {
                None
            }
        };

        // Add the updated entry to the ready list if the file is ready
        if let Some(entry) = ready_entry {
            self.push_ready(entry);
        }

        Ok(())
    }

    /// Wait for interesting events happen on the files in the interest list
    /// of the epoll file.
    ///
    /// This method blocks until either some interesting events happen or
    /// the timeout expires or a signal arrives. The first case returns
    /// `Ok(events)`, where `events` is a `Vec` containing at most `max_events`
    /// number of `EpollEvent`s. The second and third case returns errors.
    ///
    /// When `max_events` equals to zero, the method returns when the timeout
    /// expires or a signal arrives.
    pub fn wait(&self, max_events: usize, timeout: Option<&Duration>) -> Result<Vec<EpollEvent>> {
        let mut ep_events = Vec::new();
        let mut poller = None;
        loop {
            // Try to pop some ready entries
            self.pop_multi_ready(max_events, &mut ep_events);
            if !ep_events.is_empty() {
                return Ok(ep_events);
            }

            // Return immediately if specifying a timeout of zero
            if timeout.is_some() && timeout.as_ref().unwrap().is_zero() {
                return Ok(ep_events);
            }

            // If no ready entries for now, wait for them
            if poller.is_none() {
                poller = Some(Poller::new());
                let events = self.pollee.poll(IoEvents::IN, poller.as_mut());
                if !events.is_empty() {
                    continue;
                }
            }

            if let Some(timeout) = timeout {
                poller.as_ref().unwrap().wait_timeout(timeout)?;
            } else {
                poller.as_ref().unwrap().wait()?;
            }
        }
    }

    fn push_ready(&self, entry: Arc<EpollEntry>) {
        // Note that we cannot take the `EpollEntryInner` lock because we are in the callback of
        // the event observer. Doing so will cause dead locks due to inconsistent locking orders.
        //
        // We don't need to take the lock because
        // - We always call `file.poll()` immediately after calling `self.set_enabled()` and
        //   `file.register_observer()`, so all events are caught either here or by the immediate
        //   poll; in other words, we don't lose any events.
        // - Catching spurious events here is always fine because we always check them later before
        //   returning events to the user (in `EpollEntry::poll`).
        if !entry.is_enabled() {
            return;
        }

        self.ready.lock_with(|ready| {
            if !entry.is_ready() {
                entry.set_ready(ready);
                ready.push_back(Arc::downgrade(&entry));
            }

            // Even if the entry is already set to ready,
            // there might be new events that we are interested in.
            // Wake the poller anyway.
            self.pollee.add_events(IoEvents::IN);
        });
    }

    fn pop_multi_ready(&self, max_events: usize, ep_events: &mut Vec<EpollEvent>) {
        let pop_guard = self.pop_guard.lock();

        let mut limit = None;

        loop {
            if ep_events.len() >= max_events {
                break;
            }

            // Since we're holding `pop_guard`, no one else can pop the entries from the ready
            // list. This guarantees that `pop_one_ready` will pop the ready entries we see when
            // `pop_multi_ready` starts executing, so that such entries are never duplicated.
            let Some((entry, new_limit)) = self.pop_one_ready(limit, &pop_guard) else {
                break;
            };
            limit = Some(new_limit);

            // Poll the events. If the file is dead, we will remove the entry.
            let Some((ep_event, is_still_ready)) = entry.poll() else {
                // We're removing entries whose files are dead. This can only fail if user programs
                // remove the entry at the same time, and we run into some race conditions.
                //
                // However, this has very limited impact because we will never remove a wrong entry. So
                // the error can be silently ignored.
                let _ = self.del_interest(entry.fd(), entry.file_weak().clone());
                continue;
            };

            // Save the event in the output vector, if any.
            if let Some(event) = ep_event {
                ep_events.push(event);
            }

            // Add the entry back to the ready list, if necessary.
            if is_still_ready {
                self.push_ready(entry);
            }
        }
    }

    fn pop_one_ready(
        &self,
        limit: Option<usize>,
        _guard: &MutexGuard<PopGuard>,
    ) -> Option<(Arc<EpollEntry>, usize)> {
        if limit == Some(0) {
            return None;
        }

        self.ready.lock_with(|ready| {
            let mut limit = limit.unwrap_or(ready.len());

            while limit > 0 {
                limit -= 1;

                // Pop the front entry. Note that `_guard` and `limit` guarantee that this entry must
                // exist, so we can just unwrap it.
                let weak_entry = ready.pop_front().unwrap();

                // Clear the epoll file's events if there are no ready entries.
                if ready.is_empty() {
                    self.pollee.del_events(IoEvents::IN);
                }

                let Some(entry) = Weak::upgrade(&weak_entry) else {
                    // The entry has been deleted.
                    continue;
                };

                // Mark the entry as not ready. We can invoke `push_ready` later to add it back to the
                // ready list if we need to.
                entry.reset_ready(ready);

                return Some((entry, limit));
            }

            None
        })
    }

    fn warn_unsupported_flags(&self, flags: &EpollFlags) {
        if flags.intersects(EpollFlags::EXCLUSIVE | EpollFlags::WAKE_UP) {
            warn!("{:?} contains unsupported flags", flags);
        }
    }
}

impl Pollable for EpollFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
    }
}

// Implement the common methods required by FileHandle
impl FileLike for EpollFile {
    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "epoll files do not support read");
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "epoll files do not support write");
    }

    fn ioctl(&self, _cmd: IoctlCmd, _arg: usize) -> Result<i32> {
        return_errno_with_message!(Errno::EINVAL, "epoll files do not support ioctl");
    }

    fn register_observer(
        &self,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        self.pollee.register_observer(observer, mask);
        Ok(())
    }

    fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Option<Weak<dyn Observer<IoEvents>>> {
        self.pollee.unregister_observer(observer)
    }
}

/// An epoll entry that is contained in an epoll file.
///
/// Each epoll entry can be added, modified, or deleted by the `EpollCtl` command.
pub struct EpollEntry {
    // The file descriptor and the file
    key: EpollEntryKey,
    // The event masks and flags
    inner: Mutex<EpollEntryInner>,
    // Whether the entry is enabled
    is_enabled: AtomicBool,
    // Whether the entry is in the ready list
    is_ready: AtomicBool,
    // The epoll file that contains this epoll entry
    weak_epoll: Weak<EpollFile>,
    // The epoll entry itself (always inside an `Arc`)
    weak_self: Weak<Self>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct EpollEntryKey {
    fd: FileDesc,
    file: KeyableWeak<dyn FileLike>,
}

impl From<(FileDesc, KeyableWeak<dyn FileLike>)> for EpollEntryKey {
    fn from(value: (FileDesc, KeyableWeak<dyn FileLike>)) -> Self {
        Self {
            fd: value.0,
            file: value.1,
        }
    }
}

impl From<(FileDesc, &Arc<dyn FileLike>)> for EpollEntryKey {
    fn from(value: (FileDesc, &Arc<dyn FileLike>)) -> Self {
        Self {
            fd: value.0,
            file: KeyableWeak::from(Arc::downgrade(value.1)),
        }
    }
}

struct EpollEntryInner {
    event: EpollEvent,
    flags: EpollFlags,
}

impl Default for EpollEntryInner {
    fn default() -> Self {
        Self {
            event: EpollEvent {
                events: IoEvents::empty(),
                user_data: 0,
            },
            flags: EpollFlags::empty(),
        }
    }
}

impl EpollEntry {
    /// Creates a new epoll entry associated with the given epoll file.
    pub fn new(
        fd: FileDesc,
        file: KeyableWeak<dyn FileLike>,
        weak_epoll: Weak<EpollFile>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            key: EpollEntryKey { fd, file },
            inner: Mutex::new(EpollEntryInner::default()),
            is_enabled: AtomicBool::new(false),
            is_ready: AtomicBool::new(false),
            weak_epoll,
            weak_self: me.clone(),
        })
    }

    /// Get the epoll file associated with this epoll entry.
    pub fn epoll_file(&self) -> Option<Arc<EpollFile>> {
        self.weak_epoll.upgrade()
    }

    /// Get an instance of `Arc` that refers to this epoll entry.
    pub fn self_arc(&self) -> Arc<Self> {
        self.weak_self.upgrade().unwrap()
    }

    /// Get an instance of `Weak` that refers to this epoll entry.
    pub fn self_weak(&self) -> Weak<Self> {
        self.weak_self.clone()
    }

    /// Get the file associated with this epoll entry.
    ///
    /// Since an epoll entry only holds a weak reference to the file,
    /// it is possible (albeit unlikely) that the file has been dropped.
    pub fn file(&self) -> Option<Arc<dyn FileLike>> {
        self.key.file.upgrade().map(KeyableArc::into)
    }

    /// Polls the events of the file associated with this epoll entry.
    ///
    /// This method returns `None` if the file is dead. Otherwise, it returns the epoll event (if
    /// any) and a boolean value indicating whether the entry should be kept in the ready list
    /// (`true`) or removed from the ready list (`false`).
    pub fn poll(&self) -> Option<(Option<EpollEvent>, bool)> {
        let file = self.file()?;
        let inner = self.inner.lock();

        // There are no events if the entry is disabled.
        if !self.is_enabled() {
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
            self.reset_enabled(&inner);
        }

        Some((ep_event, is_still_ready))
    }

    /// Updates the epoll entry by the given event masks and flags.
    ///
    /// This method needs to be called in response to `EpollCtl::Add` and `EpollCtl::Mod`.
    pub fn update(&self, event: EpollEvent, flags: EpollFlags) -> Result<IoEvents> {
        let file = self.file().unwrap();

        let mut inner = self.inner.lock();

        file.register_observer(self.self_weak(), event.events)?;
        *inner = EpollEntryInner { event, flags };

        self.set_enabled(&inner);
        let events = file.poll(event.events, None);

        Ok(events)
    }

    /// Shuts down the epoll entry.
    ///
    /// This method needs to be called in response to `EpollCtl::Del`.
    pub fn shutdown(&self) {
        let inner = self.inner.lock();

        if let Some(file) = self.file() {
            file.unregister_observer(&(self.self_weak() as _)).unwrap();
        };
        self.reset_enabled(&inner);
    }

    /// Returns whether the epoll entry is in the ready list.
    ///
    /// *Caution:* If this method is called without holding the lock of the ready list, the user
    /// must ensure that the behavior is desired with respect to the way the ready list might be
    /// modified concurrently.
    pub fn is_ready(&self) -> bool {
        self.is_ready.load(Ordering::Relaxed)
    }

    /// Marks the epoll entry as being in the ready list.
    ///
    /// This method must be called while holding the lock of the ready list. This is the only way
    /// to ensure that the "is ready" state matches the fact that the entry is actually in the
    /// ready list.
    pub fn set_ready(&self, _guard: &mut VecDeque<Weak<EpollEntry>>) {
        self.is_ready.store(true, Ordering::Relaxed);
    }

    /// Marks the epoll entry as not being in the ready list.
    ///
    /// This method must be called while holding the lock of the ready list. This is the only way
    /// to ensure that the "is ready" state matches the fact that the entry is actually in the
    /// ready list.
    pub fn reset_ready(&self, _guard: &mut VecDeque<Weak<EpollEntry>>) {
        self.is_ready.store(false, Ordering::Relaxed)
    }

    /// Returns whether the epoll entry is enabled.
    ///
    /// *Caution:* If this method is called without holding the lock of the event masks and flags,
    /// the user must ensure that the behavior is desired with respect to the way the event masks
    /// and flags might be modified concurrently.
    pub fn is_enabled(&self) -> bool {
        self.is_enabled.load(Ordering::Relaxed)
    }

    /// Marks the epoll entry as enabled.
    ///
    /// This method must be called while holding the lock of the event masks and flags. This is the
    /// only way to ensure that the "is enabled" state describes the correct combination of the
    /// event masks and flags.
    fn set_enabled(&self, _guard: &MutexGuard<EpollEntryInner>) {
        self.is_enabled.store(true, Ordering::Relaxed)
    }

    /// Marks the epoll entry as not enabled.
    ///
    /// This method must be called while holding the lock of the event masks and flags. This is the
    /// only way to ensure that the "is enabled" state describes the correct combination of the
    /// event masks and flags.
    fn reset_enabled(&self, _guard: &MutexGuard<EpollEntryInner>) {
        self.is_enabled.store(false, Ordering::Relaxed)
    }

    /// Get the file descriptor associated with the epoll entry.
    pub fn fd(&self) -> FileDesc {
        self.key.fd
    }

    /// Get the file associated with this epoll entry.
    pub fn file_weak(&self) -> &KeyableWeak<dyn FileLike> {
        &self.key.file
    }
}

impl Observer<IoEvents> for EpollEntry {
    fn on_events(&self, _events: &IoEvents) {
        if let Some(epoll_file) = self.epoll_file() {
            epoll_file.push_ready(self.self_arc());
        }
    }
}

struct EpollEntryHolder(pub Arc<EpollEntry>);

impl PartialOrd for EpollEntryHolder {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for EpollEntryHolder {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.0.key.cmp(&other.0.key)
    }
}
impl PartialEq for EpollEntryHolder {
    fn eq(&self, other: &Self) -> bool {
        self.0.key.eq(&other.0.key)
    }
}
impl Eq for EpollEntryHolder {}

impl Borrow<EpollEntryKey> for EpollEntryHolder {
    fn borrow(&self) -> &EpollEntryKey {
        &self.0.key
    }
}

impl From<Arc<EpollEntry>> for EpollEntryHolder {
    fn from(value: Arc<EpollEntry>) -> Self {
        Self(value)
    }
}

impl Drop for EpollEntryHolder {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}
