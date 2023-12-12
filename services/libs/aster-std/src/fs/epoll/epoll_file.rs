use crate::events::{IoEvents, Observer};
use crate::fs::file_handle::FileLike;
use crate::fs::file_table::{FdEvents, FileDescripter};
use crate::fs::utils::IoctlCmd;
use crate::process::signal::{Pollee, Poller};

use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;

use super::*;

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
    interest: Mutex<BTreeMap<FileDescripter, Arc<EpollEntry>>>,
    // Entries that are probably ready (having events happened).
    ready: Mutex<VecDeque<Arc<EpollEntry>>>,
    // EpollFile itself is also pollable
    pollee: Pollee,
    // Any EpollFile is wrapped with Arc when created.
    weak_self: Weak<Self>,
}

impl EpollFile {
    /// Creates a new epoll file.
    pub fn new() -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            interest: Mutex::new(BTreeMap::new()),
            ready: Mutex::new(VecDeque::new()),
            pollee: Pollee::new(IoEvents::empty()),
            weak_self: me.clone(),
        })
    }

    /// Control the interest list of the epoll file.
    pub fn control(&self, cmd: &EpollCtl) -> Result<()> {
        match *cmd {
            EpollCtl::Add(fd, ep_event, ep_flags) => self.add_interest(fd, ep_event, ep_flags),
            EpollCtl::Del(fd) => {
                self.del_interest(fd)?;
                self.unregister_from_file_table_entry(fd);
                Ok(())
            }
            EpollCtl::Mod(fd, ep_event, ep_flags) => self.mod_interest(fd, ep_event, ep_flags),
        }
    }

    fn add_interest(
        &self,
        fd: FileDescripter,
        ep_event: EpollEvent,
        ep_flags: EpollFlags,
    ) -> Result<()> {
        self.warn_unsupported_flags(&ep_flags);

        let current = current!();
        let file_table = current.file_table().lock();
        let file_table_entry = file_table.get_entry(fd)?;
        let file = file_table_entry.file();
        let weak_file = Arc::downgrade(file);
        let mask = ep_event.events;
        let entry = EpollEntry::new(fd, weak_file, ep_event, ep_flags, self.weak_self.clone());

        // Add the new entry to the interest list and start monitering its events
        let mut interest = self.interest.lock();
        if interest.contains_key(&fd) {
            return_errno_with_message!(Errno::EEXIST, "the fd has been added");
        }
        file.register_observer(entry.self_weak() as _, IoEvents::all())?;
        interest.insert(fd, entry.clone());
        // Register self to the file table entry
        file_table_entry.register_observer(self.weak_self.clone() as _);
        let file = file.clone();
        drop(file_table);
        drop(interest);

        // Add the new entry to the ready list if the file is ready
        let events = file.poll(mask, None);
        if !events.is_empty() {
            self.push_ready(entry);
        }
        Ok(())
    }

    fn del_interest(&self, fd: FileDescripter) -> Result<()> {
        let mut interest = self.interest.lock();
        let entry = interest
            .remove(&fd)
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "fd is not in the interest list"))?;

        // If this epoll entry is in the ready list, then we should delete it.
        // But unfortunately, deleting an entry from the ready list has a
        // complexity of O(N).
        //
        // To optimize the performance, we only mark the epoll entry as
        // deleted at this moment. The real deletion happens when the ready list
        // is scanned in EpolFile::wait.
        entry.set_deleted();

        let file = match entry.file() {
            Some(file) => file,
            // TODO: should we warn about it?
            None => return Ok(()),
        };

        file.unregister_observer(&(entry.self_weak() as _)).unwrap();
        Ok(())
    }

    fn mod_interest(
        &self,
        fd: FileDescripter,
        new_ep_event: EpollEvent,
        new_ep_flags: EpollFlags,
    ) -> Result<()> {
        self.warn_unsupported_flags(&new_ep_flags);

        // Update the epoll entry
        let interest = self.interest.lock();
        let entry = interest
            .get(&fd)
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "fd is not in the interest list"))?;
        if entry.is_deleted() {
            return_errno_with_message!(Errno::ENOENT, "fd is not in the interest list");
        }
        let new_mask = new_ep_event.events;
        entry.update(new_ep_event, new_ep_flags);
        let entry = entry.clone();
        drop(interest);

        // Add the updated entry to the ready list if the file is ready
        let file = match entry.file() {
            Some(file) => file,
            None => return Ok(()),
        };
        let events = file.poll(new_mask, None);
        if !events.is_empty() {
            self.push_ready(entry);
        }
        Ok(())
    }

    fn unregister_from_file_table_entry(&self, fd: FileDescripter) {
        let current = current!();
        let file_table = current.file_table().lock();
        if let Ok(entry) = file_table.get_entry(fd) {
            entry.unregister_observer(&(self.weak_self.clone() as _));
        }
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
                let events = self.pollee.poll(IoEvents::IN, poller.as_ref());
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
        if entry.is_deleted() {
            return;
        }

        if !entry.is_ready() {
            entry.set_ready();
            ready.push_back(entry);
        }

        // Even if the entry is already set to ready, there might be new events that we are interested in.
        // Wake the poller anyway.
        self.pollee.add_events(IoEvents::IN);
    }

    fn pop_ready(&self, max_events: usize, ep_events: &mut Vec<EpollEvent>) -> usize {
        let mut count_events = 0;
        let mut ready = self.ready.lock();
        let mut pop_quota = ready.len();
        loop {
            // Pop some ready entries per round.
            //
            // Since the popped ready entries may contain "false positive" and
            // we want to return as many results as possible, this has to
            // be done in a loop.
            let pop_count = (max_events - count_events).min(pop_quota);
            if pop_count == 0 {
                break;
            }
            let ready_entries: Vec<Arc<EpollEntry>> = ready
                .drain(..pop_count)
                .filter(|entry| !entry.is_deleted())
                .collect();
            pop_quota -= pop_count;

            // Examine these ready entries, which are candidates for the results
            // to be returned.
            for entry in ready_entries {
                let (ep_event, ep_flags) = entry.event_and_flags();
                // If this entry's file is ready, save it in the output array.
                // EPOLLHUP and EPOLLERR should always be reported.
                let ready_events = entry.poll() & (ep_event.events | IoEvents::HUP | IoEvents::ERR);
                // If there are no events, the entry should be removed from the ready list.
                if ready_events.is_empty() {
                    entry.reset_ready();
                    // For EPOLLONESHOT flag, this entry should also be removed from the interest list
                    if ep_flags.intersects(EpollFlags::ONE_SHOT) {
                        self.del_interest(entry.fd())
                            .expect("this entry should be in the interest list");
                    }
                    continue;
                }

                // Records the events from the ready list
                ep_events.push(EpollEvent::new(ready_events, ep_event.user_data));
                count_events += 1;

                // If the epoll entry is neither edge-triggered or one-shot, then we should
                // keep the entry in the ready list.
                if !ep_flags.intersects(EpollFlags::ONE_SHOT | EpollFlags::EDGE_TRIGGER) {
                    ready.push_back(entry);
                }
                // Otherwise, the entry is indeed removed the ready list and we should reset
                // its ready flag.
                else {
                    entry.reset_ready();
                    // For EPOLLONESHOT flag, this entry should also be removed from the interest list
                    if ep_flags.intersects(EpollFlags::ONE_SHOT) {
                        self.del_interest(entry.fd())
                            .expect("this entry should be in the interest list");
                    }
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

impl Observer<FdEvents> for EpollFile {
    fn on_events(&self, events: &FdEvents) {
        // Delete the file from the interest list if it is closed.
        if let FdEvents::Close(fd) = events {
            let _ = self.del_interest(*fd);
        }
    }
}

impl Drop for EpollFile {
    fn drop(&mut self) {
        trace!("EpollFile Drop");
        let mut interest = self.interest.lock();
        let fds: Vec<_> = interest
            .extract_if(|_, _| true)
            .map(|(fd, entry)| {
                entry.set_deleted();
                if let Some(file) = entry.file() {
                    let _ = file.unregister_observer(&(entry.self_weak() as _));
                }
                fd
            })
            .collect();
        drop(interest);

        fds.iter()
            .for_each(|&fd| self.unregister_from_file_table_entry(fd));
    }
}

// Implement the common methods required by FileHandle
impl FileLike for EpollFile {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "epoll files do not support read");
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "epoll files do not support write");
    }

    fn ioctl(&self, _cmd: IoctlCmd, _arg: usize) -> Result<i32> {
        return_errno_with_message!(Errno::EINVAL, "epoll files do not support ioctl");
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.pollee.poll(mask, poller)
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
    ) -> Result<Weak<dyn Observer<IoEvents>>> {
        self.pollee
            .unregister_observer(observer)
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "observer is not registered"))
    }
}

/// An epoll entry contained in an epoll file. Each epoll entry is added, modified,
/// or deleted by the `EpollCtl` command.
pub struct EpollEntry {
    fd: FileDescripter,
    file: Weak<dyn FileLike>,
    inner: Mutex<Inner>,
    // Whether the entry is in the ready list
    is_ready: AtomicBool,
    // Whether the entry has been deleted from the interest list
    is_deleted: AtomicBool,
    // Refers to the epoll file containing this epoll entry
    weak_epoll: Weak<EpollFile>,
    // An EpollEntry is always contained inside Arc
    weak_self: Weak<Self>,
}

struct Inner {
    event: EpollEvent,
    flags: EpollFlags,
}

impl EpollEntry {
    /// Creates a new epoll entry associated with the given epoll file.
    pub fn new(
        fd: FileDescripter,
        file: Weak<dyn FileLike>,
        event: EpollEvent,
        flags: EpollFlags,
        weak_epoll: Weak<EpollFile>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            fd,
            file,
            inner: Mutex::new(Inner { event, flags }),
            is_ready: AtomicBool::new(false),
            is_deleted: AtomicBool::new(false),
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
        self.file.upgrade()
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
        match self.file.upgrade() {
            Some(file) => file.poll(IoEvents::all(), None),
            None => IoEvents::empty(),
        }
    }

    /// Update the epoll entry, most likely to be triggered via `EpollCtl::Mod`.
    pub fn update(&self, event: EpollEvent, flags: EpollFlags) {
        let mut inner = self.inner.lock();
        *inner = Inner { event, flags }
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

    /// Returns whether the epoll entry has been deleted from the interest list.
    pub fn is_deleted(&self) -> bool {
        self.is_deleted.load(Ordering::Relaxed)
    }

    /// Mark the epoll entry as having been deleted from the interest list.
    pub fn set_deleted(&self) {
        self.is_deleted.store(true, Ordering::Relaxed);
    }

    /// Get the file descriptor associated with the epoll entry.
    pub fn fd(&self) -> FileDescripter {
        self.fd
    }
}

impl Observer<IoEvents> for EpollEntry {
    fn on_events(&self, _events: &IoEvents) {
        // Fast path
        if self.is_deleted() {
            return;
        }

        if let Some(epoll_file) = self.epoll_file() {
            epoll_file.push_ready(self.self_arc());
        }
    }
}
