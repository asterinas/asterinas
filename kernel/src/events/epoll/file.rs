// SPDX-License-Identifier: MPL-2.0

use core::{borrow::Borrow, fmt::Display, time::Duration};

use keyable_arc::KeyableWeak;

use super::{
    EpollCtl, EpollEvent, EpollFlags,
    entry::{Entry, EntryKey, ReadySet},
};
use crate::{
    events::IoEvents,
    fs::{
        file::{
            AccessMode, CreationFlags, FileLike,
            file_table::{FdFlags, FileDesc, get_file_fast},
        },
        pseudofs::AnonInodeFs,
        vfs::path::Path,
    },
    prelude::*,
    process::{
        posix_thread::ThreadLocal,
        signal::{PollHandle, Pollable},
    },
    util::ioctl::RawIoctl,
};

/// Global mutex to prevent parallel cycle formation in epoll topologies.
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/fs/eventpoll.c#L258>
static EPNESTED_MUTEX: Mutex<()> = Mutex::new(());

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
    /// The pseudo path associated with this epoll file.
    pseudo_path: Path,
    /// Serializes epoll_ctl operations (check + insert) to prevent TOCTOU races.
    /// Reference: <https://elixir.bootlin.com/linux/v6.18/source/fs/eventpoll.c#L2316>
    mtx: Mutex<()>,
}

/// Result of the reachability check.
enum ReachResult {
    /// Cycle detected.
    Found,
    /// Nesting depth exceeded limit.
    TooDeep,
    /// Not found.
    NotFound,
}

impl EpollFile {
    /// Creates a new epoll file.
    pub fn new() -> Arc<Self> {
        let pseudo_path = AnonInodeFs::new_path(|_| "anon_inode:[eventpoll]".to_string());

        Arc::new(Self {
            interest: Mutex::new(BTreeSet::new()),
            ready: Arc::new(ReadySet::new()),
            pseudo_path,
            mtx: Mutex::new(()),
        })
    }

    /// Controls the interest list of the epoll file.
    /// Reference: <https://elixir.bootlin.com/linux/v6.18/source/fs/eventpoll.c#L2316>
    pub fn control(&self, thread_local: &ThreadLocal, cmd: &EpollCtl) -> Result<()> {
        let fd = match cmd {
            EpollCtl::Add(fd, ..) => *fd,
            EpollCtl::Del(fd) => *fd,
            EpollCtl::Mod(fd, ..) => *fd,
        };

        let mut file_table = thread_local.borrow_file_table_mut();
        let file = get_file_fast!(&mut file_table, fd).into_owned();
        drop(file_table);

        // Reject cycles: an epoll file cannot monitor itself or form indirect cycles.
        if let EpollCtl::Add(..) = cmd {
            if (self as *const _ as *const ()) == (file.as_ref() as *const _ as *const ()) {
                return_errno_with_message!(Errno::EINVAL, "epoll file cannot be added to itself");
            }
            if let Some(target_epoll) = file.downcast_ref::<EpollFile>() {
                // Lock ordering: epnested_mutex -> mtx
                let _epnested_guard = EPNESTED_MUTEX.lock();
                self.check_cycle(target_epoll)?;

                let _mtx_guard = self.mtx.lock();
                let EpollCtl::Add(fd, ep_event, ep_flags) = *cmd else {
                    unreachable!()
                };
                return self.add_interest(fd, file, ep_event, ep_flags);
            }
        }

        // For non-epoll ADD or DEL/MOD operations
        let _mtx_guard = self.mtx.lock();

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

    /// Maximum number of nesting allowed inside epoll sets.
    /// Reference: <https://elixir.bootlin.com/linux/v6.18/source/fs/eventpoll.c#L94>
    const EP_MAX_NESTS: usize = 4;

    /// Checks whether adding `target` as an interest of `self` would create a cycle
    /// or exceed the maximum nesting depth.
    /// Reference: <https://elixir.bootlin.com/linux/v6.18/source/fs/eventpoll.c#L2132>
    fn check_cycle(&self, target: &EpollFile) -> Result<()> {
        let self_ptr = self as *const Self as *const ();
        let mut visited = Vec::new();
        match target.can_reach(self_ptr, &mut visited, 0) {
            ReachResult::Found => {
                return_errno_with_message!(
                    Errno::ELOOP,
                    "adding this fd would create an epoll cycle"
                );
            }
            ReachResult::TooDeep => {
                return_errno_with_message!(
                    Errno::ELOOP,
                    "adding this fd would exceed the maximum epoll nesting depth"
                );
            }
            ReachResult::NotFound => Ok(()),
        }
    }

    /// Checks whether an epoll file at the given raw pointer is reachable by
    /// recursively traversing the epoll files monitored by `self`.
    fn can_reach(
        &self,
        target_ptr: *const (),
        visited: &mut Vec<*const ()>,
        depth: usize,
    ) -> ReachResult {
        let self_ptr = self as *const Self as *const ();

        if self_ptr == target_ptr {
            return ReachResult::Found;
        }

        if depth > Self::EP_MAX_NESTS {
            return ReachResult::TooDeep;
        }

        if visited.contains(&self_ptr) {
            return ReachResult::NotFound;
        }

        visited.push(self_ptr);

        let files: Vec<Arc<dyn FileLike>> = {
            let interest = self.interest.lock();
            interest.iter().filter_map(|entry| entry.0.file()).collect()
        };

        let mut max_depth_reached = false;
        for file in &files {
            if let Some(ep) = file.downcast_ref::<EpollFile>() {
                let ep_ptr = ep as *const EpollFile as *const ();
                if ep_ptr == target_ptr {
                    return ReachResult::Found;
                }
                match ep.can_reach(target_ptr, visited, depth + 1) {
                    ReachResult::Found => return ReachResult::Found,
                    ReachResult::TooDeep => max_depth_reached = true,
                    ReachResult::NotFound => {}
                }
            }
        }

        if max_depth_reached {
            ReachResult::TooDeep
        } else {
            ReachResult::NotFound
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
    fn ioctl(&self, _raw_ioctl: RawIoctl) -> Result<i32> {
        return_errno_with_message!(Errno::ENOTTY, "epoll files do not support ioctl");
    }

    fn access_mode(&self) -> AccessMode {
        // Reference: <https://elixir.bootlin.com/linux/v7.0/source/fs/eventpoll.c#L2191>.
        AccessMode::O_RDWR
    }

    fn path(&self) -> &Path {
        &self.pseudo_path
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            inner: Arc<EpollFile>,
            fd_flags: FdFlags,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut flags = self.inner.status_flags().bits() | self.inner.access_mode() as u32;
                if self.fd_flags.contains(FdFlags::CLOEXEC) {
                    flags |= CreationFlags::O_CLOEXEC.bits();
                }

                writeln!(f, "pos:\t{}", 0)?;
                writeln!(f, "flags:\t0{:o}", flags)?;
                writeln!(f, "mnt_id:\t{}", AnonInodeFs::mount_node().id())?;
                writeln!(f, "ino:\t{}", AnonInodeFs::shared_inode().ino())?;
                for entry in self.inner.interest.lock().iter() {
                    writeln!(f, "{}", entry.0)?;
                }

                Ok(())
            }
        }

        Box::new(FdInfo {
            inner: self,
            fd_flags,
        })
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
