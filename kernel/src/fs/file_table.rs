// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use core::sync::atomic::{AtomicU8, Ordering};

use aster_util::slot_vec::SlotVec;

use super::{
    file_handle::FileLike,
    fs_resolver::{FsPath, FsResolver, AT_FDCWD},
    inode_handle::InodeHandle,
    utils::{AccessMode, InodeMode},
};
use crate::{
    events::{Events, IoEvents, Observer, Subject},
    fs::utils::StatusFlags,
    net::socket::Socket,
    prelude::*,
    process::{
        signal::{constants::SIGIO, signals::kernel::KernelSignal},
        Pid, Process,
    },
};

pub type FileDesc = i32;

pub struct FileTable {
    table: SlotVec<FileTableEntry>,
    subject: Subject<FdEvents>,
}

impl FileTable {
    pub const fn new() -> Self {
        Self {
            table: SlotVec::new(),
            subject: Subject::new(),
        }
    }

    pub fn new_with_stdio() -> Self {
        let mut table = SlotVec::new();
        let fs_resolver = FsResolver::new();
        let tty_path = FsPath::new(AT_FDCWD, "/dev/console").expect("cannot find tty");
        let stdin = {
            let flags = AccessMode::O_RDONLY as u32;
            let mode = InodeMode::S_IRUSR;
            fs_resolver.open(&tty_path, flags, mode.bits()).unwrap()
        };
        let stdout = {
            let flags = AccessMode::O_WRONLY as u32;
            let mode = InodeMode::S_IWUSR;
            fs_resolver.open(&tty_path, flags, mode.bits()).unwrap()
        };
        let stderr = {
            let flags = AccessMode::O_WRONLY as u32;
            let mode = InodeMode::S_IWUSR;
            fs_resolver.open(&tty_path, flags, mode.bits()).unwrap()
        };
        table.put(FileTableEntry::new(Arc::new(stdin), FdFlags::empty()));
        table.put(FileTableEntry::new(Arc::new(stdout), FdFlags::empty()));
        table.put(FileTableEntry::new(Arc::new(stderr), FdFlags::empty()));
        Self {
            table,
            subject: Subject::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.table.slots_len()
    }

    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    pub fn dup(&mut self, fd: FileDesc, new_fd: FileDesc, flags: FdFlags) -> Result<FileDesc> {
        let file = self
            .table
            .get(fd as usize)
            .map(|entry| {
                entry
                    .file
                    .as_ref()
                    .expect(FILE_IS_NONE_ERROR_MESSAGE)
                    .clone()
            })
            .ok_or(Error::with_message(Errno::ENOENT, "No such file"))?;

        // Get the lowest-numbered available fd equal to or greater than `new_fd`.
        let get_min_free_fd = || -> usize {
            let new_fd = new_fd as usize;
            if self.table.get(new_fd).is_none() {
                return new_fd;
            }

            for idx in new_fd + 1..self.len() {
                if self.table.get(idx).is_none() {
                    return idx;
                }
            }
            self.len()
        };

        let min_free_fd = get_min_free_fd();
        let entry = FileTableEntry::new(file, flags);
        self.table.put_at(min_free_fd, entry);
        Ok(min_free_fd as FileDesc)
    }

    pub fn insert(&mut self, item: Arc<dyn FileLike>, flags: FdFlags) -> FileDesc {
        let entry = FileTableEntry::new(item, flags);
        self.table.put(entry) as FileDesc
    }

    pub fn insert_at(
        &mut self,
        fd: FileDesc,
        item: Arc<dyn FileLike>,
        flags: FdFlags,
    ) -> Option<Arc<dyn FileLike>> {
        let entry = FileTableEntry::new(item, flags);
        let entry = self.table.put_at(fd as usize, entry);
        if entry.is_some() {
            let events = FdEvents::Close(fd);
            self.notify_fd_events(&events);
            entry.as_ref().unwrap().notify_fd_events(&events);
        }
        entry.map(|e| e.file.expect(FILE_IS_NONE_ERROR_MESSAGE))
    }

    pub fn close_file(&mut self, fd: FileDesc) -> Option<Arc<dyn FileLike>> {
        let removed_entry = self.table.remove(fd as usize)?;

        let events = FdEvents::Close(fd);
        self.notify_fd_events(&events);
        removed_entry.notify_fd_events(&events);

        let closed_file = removed_entry.file.expect(FILE_IS_NONE_ERROR_MESSAGE);
        if let Some(closed_inode_file) = closed_file.downcast_ref::<InodeHandle>() {
            // FIXME: Operation below should not hold any mutex if `self` is protected by a spinlock externally
            closed_inode_file.release_range_locks();
        }
        Some(closed_file)
    }

    pub fn close_all(&mut self) -> Vec<Arc<dyn FileLike>> {
        self.close_files(|_, _| true)
    }

    pub fn close_files_on_exec(&mut self) -> Vec<Arc<dyn FileLike>> {
        self.close_files(|_, entry| entry.flags().contains(FdFlags::CLOEXEC))
    }

    fn close_files<F>(&mut self, should_close: F) -> Vec<Arc<dyn FileLike>>
    where
        F: Fn(FileDesc, &FileTableEntry) -> bool,
    {
        let mut closed_files = Vec::new();
        let closed_fds: Vec<FileDesc> = self
            .table
            .idxes_and_items()
            .filter_map(|(idx, entry)| {
                if should_close(idx as FileDesc, entry) {
                    Some(idx as FileDesc)
                } else {
                    None
                }
            })
            .collect();

        for fd in closed_fds {
            let removed_entry = self.table.remove(fd as usize).unwrap();
            let events = FdEvents::Close(fd);
            self.notify_fd_events(&events);
            removed_entry.notify_fd_events(&events);
            closed_files.push(
                removed_entry
                    .file
                    .as_ref()
                    .expect(FILE_IS_NONE_ERROR_MESSAGE)
                    .clone(),
            );
            if let Some(inode_file) = removed_entry
                .file
                .expect(FILE_IS_NONE_ERROR_MESSAGE)
                .downcast_ref::<InodeHandle>()
            {
                // FIXME: Operation below should not hold any mutex if `self` is protected by a spinlock externally
                inode_file.release_range_locks();
            }
        }

        closed_files
    }

    pub fn get_file(&self, fd: FileDesc) -> Result<&Arc<dyn FileLike>> {
        self.table
            .get(fd as usize)
            .map(|entry| entry.file.as_ref().expect(FILE_IS_NONE_ERROR_MESSAGE))
            .ok_or(Error::with_message(Errno::EBADF, "fd not exits"))
    }

    pub fn get_socket(&self, sockfd: FileDesc) -> Result<Arc<dyn Socket>> {
        let file_like = self.get_file(sockfd)?.clone();
        file_like
            .as_socket()
            .ok_or_else(|| Error::with_message(Errno::ENOTSOCK, "the fd is not a socket"))
    }

    pub fn get_entry(&self, fd: FileDesc) -> Result<&FileTableEntry> {
        self.table
            .get(fd as usize)
            .ok_or(Error::with_message(Errno::EBADF, "fd not exits"))
    }

    pub fn get_entry_mut(&mut self, fd: FileDesc) -> Result<&mut FileTableEntry> {
        self.table
            .get_mut(fd as usize)
            .ok_or(Error::with_message(Errno::EBADF, "fd not exits"))
    }

    pub fn fds_and_files(&self) -> impl Iterator<Item = (FileDesc, &'_ Arc<dyn FileLike>)> {
        self.table.idxes_and_items().map(|(idx, entry)| {
            (
                idx as FileDesc,
                entry.file.as_ref().expect(FILE_IS_NONE_ERROR_MESSAGE),
            )
        })
    }

    pub fn register_observer(&self, observer: Weak<dyn Observer<FdEvents>>) {
        self.subject.register_observer(observer, ());
    }

    pub fn unregister_observer(&self, observer: &Weak<dyn Observer<FdEvents>>) {
        self.subject.unregister_observer(observer);
    }

    fn notify_fd_events(&self, events: &FdEvents) {
        self.subject.notify_observers(events);
    }

    /// Temporarily takes the file out from the file table.
    ///
    /// This API is currently intended solely for use with the `poll` syscall.
    /// The taken file must be returned by invoking [`Self::insert_file_like`] afterwards.
    ///
    /// The user of this API must ensure that it has exclusive access to the file table
    /// between calling this method and [`Self::insert_file_like`].
    /// Failure to do so may result in a data race.
    pub fn take_file_like(&mut self, fd: FileDesc) -> Option<Arc<dyn FileLike>> {
        self.table.get_mut(fd as usize)?.file.take()
    }

    /// Inserts a file back into the file table.
    ///
    /// This method must be used in conjunction with [`Self::take_file_like`].
    pub fn insert_file_like(&mut self, fd: FileDesc, file_like: Arc<dyn FileLike>) {
        let _ = self
            .table
            .get_mut(fd as usize)
            .unwrap()
            .file
            .insert(file_like);
    }
}

impl Default for FileTable {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for FileTable {
    fn clone(&self) -> Self {
        Self {
            table: self.table.clone(),
            subject: Subject::new(),
        }
    }
}

impl Drop for FileTable {
    fn drop(&mut self) {
        let events = FdEvents::DropFileTable;
        self.subject.notify_observers(&events);
    }
}

#[derive(Copy, Clone, Debug)]
pub enum FdEvents {
    Close(FileDesc),
    DropFileTable,
}

impl Events for FdEvents {}

pub struct FileTableEntry {
    // Explanation of why the file can be `None`:
    //
    // The `file` may be `None` because the `Arc<dyn FileLike>` can be temporarily
    // taken when performing a poll operation.
    // This approach offers a performance benefit:
    // taking and re-inserting the `Arc<dyn FileLike>` is more efficient than cloning the Arc.
    // In most cases, the file can be considered `Some(_)`,
    // as the poll operation will only take files
    // when the file table can be viewed as exclusively owned by the current thread.
    // For further details, refer to `crate::syscall::poll::hold_files`.
    file: Option<Arc<dyn FileLike>>,
    flags: AtomicU8,
    subject: Subject<FdEvents>,
    owner: Option<Owner>,
}

const FILE_IS_NONE_ERROR_MESSAGE: &str = "[Internal Error] You are trying to get the `Arc<FileLike>`, which is temporarily taken during polling.";

impl FileTableEntry {
    pub fn new(file: Arc<dyn FileLike>, flags: FdFlags) -> Self {
        Self {
            file: Some(file),
            flags: AtomicU8::new(flags.bits()),
            subject: Subject::new(),
            owner: None,
        }
    }

    pub fn file(&self) -> &Arc<dyn FileLike> {
        self.file.as_ref().expect(FILE_IS_NONE_ERROR_MESSAGE)
    }

    pub fn owner(&self) -> Option<Pid> {
        self.owner.as_ref().map(|(pid, _)| *pid)
    }

    /// Set a process (group) as owner of the file descriptor.
    ///
    /// Such that this process (group) will receive `SIGIO` and `SIGURG` signals
    /// for I/O events on the file descriptor, if `O_ASYNC` status flag is set
    /// on this file.
    pub fn set_owner(&mut self, owner: Option<&Arc<Process>>) -> Result<()> {
        match owner {
            None => {
                // Unset the owner if the given pid is zero
                if let Some((_, observer)) = self.owner.as_ref() {
                    let _ = self
                        .file
                        .as_ref()
                        .expect(FILE_IS_NONE_ERROR_MESSAGE)
                        .unregister_observer(&Arc::downgrade(observer));
                }
                let _ = self.owner.take();
            }
            Some(owner_process) => {
                let owner_pid = owner_process.pid();
                if let Some((pid, observer)) = self.owner.as_ref() {
                    if *pid == owner_pid {
                        return Ok(());
                    }

                    let _ = self
                        .file
                        .as_ref()
                        .expect(FILE_IS_NONE_ERROR_MESSAGE)
                        .unregister_observer(&Arc::downgrade(observer));
                }

                let observer = OwnerObserver::new(
                    self.file
                        .as_ref()
                        .expect(FILE_IS_NONE_ERROR_MESSAGE)
                        .clone(),
                    Arc::downgrade(owner_process),
                );
                self.file
                    .as_ref()
                    .expect(FILE_IS_NONE_ERROR_MESSAGE)
                    .register_observer(observer.weak_self(), IoEvents::empty())?;
                let _ = self.owner.insert((owner_pid, observer));
            }
        }
        Ok(())
    }

    pub fn flags(&self) -> FdFlags {
        FdFlags::from_bits(self.flags.load(Ordering::Relaxed)).unwrap()
    }

    pub fn set_flags(&self, flags: FdFlags) {
        self.flags.store(flags.bits(), Ordering::Relaxed);
    }

    pub fn clear_flags(&self) {
        self.flags.store(0, Ordering::Relaxed);
    }

    pub fn register_observer(&self, epoll: Weak<dyn Observer<FdEvents>>) {
        self.subject.register_observer(epoll, ());
    }

    pub fn unregister_observer(&self, epoll: &Weak<dyn Observer<FdEvents>>) {
        self.subject.unregister_observer(epoll);
    }

    pub fn notify_fd_events(&self, events: &FdEvents) {
        self.subject.notify_observers(events);
    }
}

impl Clone for FileTableEntry {
    fn clone(&self) -> Self {
        Self {
            file: self.file.clone(),
            flags: AtomicU8::new(self.flags.load(Ordering::Relaxed)),
            subject: Subject::new(),
            owner: self.owner.clone(),
        }
    }
}

bitflags! {
    pub struct FdFlags: u8 {
        /// Close on exec
        const CLOEXEC = 1;
    }
}

type Owner = (Pid, Arc<dyn Observer<IoEvents>>);

struct OwnerObserver {
    file: Arc<dyn FileLike>,
    owner: Weak<Process>,
    weak_self: Weak<Self>,
}

impl OwnerObserver {
    pub fn new(file: Arc<dyn FileLike>, owner: Weak<Process>) -> Arc<Self> {
        Arc::new_cyclic(|weak_ref| Self {
            file,
            owner,
            weak_self: weak_ref.clone(),
        })
    }

    pub fn weak_self(&self) -> Weak<Self> {
        self.weak_self.clone()
    }
}

impl Observer<IoEvents> for OwnerObserver {
    fn on_events(&self, events: &IoEvents) {
        if self.file.status_flags().contains(StatusFlags::O_ASYNC)
            && let Some(process) = self.owner.upgrade()
        {
            process.enqueue_signal(KernelSignal::new(SIGIO));
        }
    }
}
