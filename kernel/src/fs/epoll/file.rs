// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::btree_set::BTreeSet, sync::Arc};
use core::{borrow::Borrow, time::Duration};

use keyable_arc::KeyableWeak;
use ostd::sync::Mutex;

use super::{
    entry::{Entry, EntryKey, ReadySet},
    EpollCtl, EpollEvent, EpollFlags,
};
use crate::{
    events::IoEvents,
    fs::{
        file_handle::FileLike,
        file_table::FileDesc,
        utils::{InodeMode, IoctlCmd, Metadata},
    },
    prelude::*,
    process::{
        posix_thread::AsPosixThread,
        signal::{PollHandle, Pollable},
    },
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
    interest: Mutex<BTreeSet<EntryHolder>>,
    // A set of ready entries.
    //
    // Keep this in a separate `Arc` to avoid dropping `EpollFile` in the observer callback, which
    // may cause deadlocks.
    ready: Arc<ReadySet>,
}

impl EpollFile {
    /// Creates a new epoll file.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            interest: Mutex::new(BTreeSet::new()),
            ready: Arc::new(ReadySet::new()),
        })
    }

    /// Controls the interest list of the epoll file.
    pub fn control(&self, cmd: &EpollCtl) -> Result<()> {
        let fd = match cmd {
            EpollCtl::Add(fd, ..) => *fd,
            EpollCtl::Del(fd) => *fd,
            EpollCtl::Mod(fd, ..) => *fd,
        };

        let file = {
            let current = current_thread!();
            let current = current.as_posix_thread().unwrap();
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
        let ready_entry = {
            let mut interest = self.interest.lock();

            if interest.contains(&EntryKey::from((fd, &file))) {
                return_errno_with_message!(
                    Errno::EEXIST,
                    "the file is already in the interest list"
                );
            }

            let entry = Entry::new(fd, Arc::downgrade(&file).into(), self.ready.clone());
            let events = entry.update(ep_event, ep_flags);

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
            self.ready.push(entry.observer());
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

        if !self.interest.lock().remove(&EntryKey::from((fd, file))) {
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

            let EntryHolder(entry) =
                interest.get(&EntryKey::from((fd, &file))).ok_or_else(|| {
                    Error::with_message(Errno::ENOENT, "the file is not in the interest list")
                })?;
            let events = entry.update(new_ep_event, new_ep_flags);

            if !events.is_empty() {
                Some(entry.clone())
            } else {
                None
            }
        };

        // Add the updated entry to the ready list if the file is ready
        if let Some(entry) = ready_entry {
            self.ready.push(entry.observer());
        }

        Ok(())
    }

    /// Waits for interesting events happen on the files in the interest list
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

        self.wait_events(IoEvents::IN, timeout, || {
            self.pop_multi_ready(max_events, &mut ep_events);

            if ep_events.is_empty() {
                return Err(Error::with_message(
                    Errno::EAGAIN,
                    "there are no available events",
                ));
            }

            Ok(())
        })?;

        Ok(ep_events)
    }

    fn pop_multi_ready(&self, max_events: usize, ep_events: &mut Vec<EpollEvent>) {
        let mut pop_iter = self.ready.lock_pop();

        loop {
            if ep_events.len() >= max_events {
                break;
            }

            // Since we're holding `pop_guard` (in `pop_iter`), no one else can pop the entries
            // from the ready list. This guarantees that `next` will pop the ready entries we see
            // when `pop_multi_ready` starts executing, so that such entries are never duplicated.
            let Some(entry) = pop_iter.next() else {
                break;
            };

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
                self.ready.push(entry.observer());
            }
        }
    }

    fn warn_unsupported_flags(&self, flags: &EpollFlags) {
        if flags.intersects(EpollFlags::EXCLUSIVE | EpollFlags::WAKE_UP) {
            warn!("{:?} contains unsupported flags", flags);
        }
    }
}

impl Pollable for EpollFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.ready.poll(mask, poller)
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

    fn metadata(&self) -> Metadata {
        // This is a dummy implementation.
        // TODO: Add "anonymous inode fs" and link `EpollFile` to it.
        Metadata::new_file(
            0,
            InodeMode::from_bits_truncate(0o600),
            aster_block::BLOCK_SIZE,
        )
    }
}

struct EntryHolder(Arc<Entry>);

impl PartialOrd for EntryHolder {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for EntryHolder {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.0.key().cmp(other.0.key())
    }
}
impl PartialEq for EntryHolder {
    fn eq(&self, other: &Self) -> bool {
        self.0.key().eq(other.0.key())
    }
}
impl Eq for EntryHolder {}

impl Borrow<EntryKey> for EntryHolder {
    fn borrow(&self) -> &EntryKey {
        self.0.key()
    }
}

impl From<Arc<Entry>> for EntryHolder {
    fn from(value: Arc<Entry>) -> Self {
        Self(value)
    }
}

impl Drop for EntryHolder {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}
