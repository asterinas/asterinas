// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]
#![allow(unused_variables)]

use core::{
    borrow::Borrow,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use keyable_arc::{KeyableArc, KeyableWeak};

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
    ready: Mutex<VecDeque<Weak<EpollEntry>>>,
    // EpollFile itself is also pollable
    pollee: Pollee,
    // Any EpollFile is wrapped with Arc when created.
    weak_self: Weak<Self>,
}

impl EpollFile {
    /// Creates a new epoll file.
    pub fn new() -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            interest: Mutex::new(BTreeSet::new()),
            ready: Mutex::new(VecDeque::new()),
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
            let file_table = current.file_table().lock();
            file_table.get_file(fd)?.clone()
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
        let entry = {
            let mut interest = self.interest.lock();

            if interest.contains(&EpollEntryKey::from((fd, &file))) {
                return_errno_with_message!(Errno::EEXIST, "the fd has been added");
            }

            let entry = EpollEntry::new(fd, &file, ep_event, ep_flags, self.weak_self.clone())?;
            let inserted = interest.insert(entry.clone().into());
            assert!(inserted);

            entry
        };

        // Add the new entry to the ready list if the file is ready
        let events = file.poll(ep_event.events, None);
        if !events.is_empty() {
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
            return_errno_with_message!(Errno::ENOENT, "fd is not in the interest list");
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
        let entry = {
            let interest = self.interest.lock();

            let EpollEntryHolder(entry) = interest
                .get(&EpollEntryKey::from((fd, &file)))
                .ok_or_else(|| {
                    Error::with_message(Errno::ENOENT, "fd is not in the interest list")
                })?;
            entry.update(new_ep_event, new_ep_flags);

            entry.clone()
        };

        // Add the updated entry to the ready list if the file is ready
        let events = file.poll(new_ep_event.events, None);
        if !events.is_empty() {
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
            if self.pop_ready(max_events, &mut ep_events) > 0 {
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
        let mut ready = self.ready.lock();

        if !entry.is_ready() {
            entry.set_ready();
            ready.push_back(Arc::downgrade(&entry));
        }

        // Even if the entry is already set to ready, there might be new events that we are interested in.
        // Wake the poller anyway.
        self.pollee.add_events(IoEvents::IN);
    }

    fn pop_ready(&self, max_events: usize, ep_events: &mut Vec<EpollEvent>) -> usize {
        let mut ready = self.ready.lock();

        let mut count_events = 0;
        for _ in 0..ready.len() {
            if count_events >= max_events {
                break;
            }

            let weak_entry = ready.pop_front().unwrap();
            let Some(entry) = Weak::upgrade(&weak_entry) else {
                // The entry has been deleted.
                continue;
            };

            let (ep_event, ep_flags) = entry.event_and_flags();
            // If this entry's file is ready, save it in the output array.
            // EPOLLHUP and EPOLLERR should always be reported.
            let ready_events = entry.poll() & (ep_event.events | IoEvents::HUP | IoEvents::ERR);

            // Records the events from the ready list
            if !ready_events.is_empty() {
                ep_events.push(EpollEvent::new(ready_events, ep_event.user_data));
                count_events += 1;
            }

            // If there are events and the epoll entry is neither edge-triggered
            // nor one-shot, then we should keep the entry in the ready list.
            if !ready_events.is_empty()
                && !ep_flags.intersects(EpollFlags::ONE_SHOT | EpollFlags::EDGE_TRIGGER)
            {
                ready.push_back(weak_entry);
            }
            // Otherwise, the entry is indeed removed the ready list and we should reset
            // its ready flag.
            else {
                entry.reset_ready();
                // For EPOLLONESHOT flag, this entry should also be removed from the interest list
                if ep_flags.intersects(EpollFlags::ONE_SHOT) {
                    // FIXME: This may fail due to race conditions.
                    let _ = self.del_interest(entry.fd(), entry.file_weak().clone());
                }
            }
        }

        // Clear the epoll file's events if no ready entries
        if ready.len() == 0 {
            self.pollee.del_events(IoEvents::IN);
        }

        count_events
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
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "epoll files do not support read");
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
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

impl EpollEntry {
    /// Creates a new epoll entry associated with the given epoll file.
    pub fn new(
        fd: FileDesc,
        file: &Arc<dyn FileLike>,
        event: EpollEvent,
        flags: EpollFlags,
        weak_epoll: Weak<EpollFile>,
    ) -> Result<Arc<Self>> {
        let entry = Arc::new_cyclic(|me| Self {
            key: EpollEntryKey {
                fd,
                file: Arc::downgrade(file).into(),
            },
            inner: Mutex::new(EpollEntryInner { event, flags }),
            is_ready: AtomicBool::new(false),
            weak_epoll,
            weak_self: me.clone(),
        });

        file.register_observer(entry.weak_self.clone(), event.events)?;

        Ok(entry)
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

    /// Get the epoll event associated with the epoll entry.
    pub fn event(&self) -> EpollEvent {
        let inner = self.inner.lock();
        inner.event
    }

    /// Get the epoll flags associated with the epoll entry.
    pub fn flags(&self) -> EpollFlags {
        let inner = self.inner.lock();
        inner.flags
    }

    /// Get the epoll event and flags that are associated with this epoll entry.
    pub fn event_and_flags(&self) -> (EpollEvent, EpollFlags) {
        let inner = self.inner.lock();
        (inner.event, inner.flags)
    }

    /// Poll the events of the file associated with this epoll entry.
    ///
    /// If the returned events is not empty, then the file is considered ready.
    pub fn poll(&self) -> IoEvents {
        match self.file() {
            Some(file) => file.poll(IoEvents::all(), None),
            None => IoEvents::empty(),
        }
    }

    /// Update the epoll entry, most likely to be triggered via `EpollCtl::Mod`.
    pub fn update(&self, event: EpollEvent, flags: EpollFlags) {
        let mut inner = self.inner.lock();
        *inner = EpollEntryInner { event, flags }
    }

    /// Returns whether the epoll entry is in the ready list.
    pub fn is_ready(&self) -> bool {
        self.is_ready.load(Ordering::Relaxed)
    }

    /// Mark the epoll entry as being in the ready list.
    pub fn set_ready(&self) {
        self.is_ready.store(true, Ordering::Relaxed);
    }

    /// Mark the epoll entry as not being in the ready list.
    pub fn reset_ready(&self) {
        self.is_ready.store(false, Ordering::Relaxed)
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
        let Some(file) = self.file() else {
            return;
        };
        file.unregister_observer(&(self.self_weak() as _)).unwrap();
    }
}
