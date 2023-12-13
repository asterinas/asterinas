// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU8, Ordering};

use aster_util::slot_vec::SlotVec;

use super::{
    file_handle::FileLike,
    fs_resolver::{FsPath, FsResolver, AT_FDCWD},
    utils::{AccessMode, InodeMode},
};
use crate::{
    events::{Events, Observer, Subject},
    net::socket::Socket,
    prelude::*,
};

pub type FileDescripter = i32;

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

    pub fn dup(
        &mut self,
        fd: FileDescripter,
        new_fd: FileDescripter,
        flags: FdFlags,
    ) -> Result<FileDescripter> {
        let file = self
            .table
            .get(fd as usize)
            .map(|entry| entry.file.clone())
            .ok_or(Error::with_message(Errno::ENOENT, "No such file"))?;

        // Get the lowest-numbered available fd equal to or greater than `new_fd`.
        let get_min_free_fd = || -> usize {
            let new_fd = new_fd as usize;
            if self.table.get(new_fd).is_none() {
                return new_fd;
            }

            for idx in new_fd + 1..self.table.slots_len() {
                if self.table.get(idx).is_none() {
                    return idx;
                }
            }
            self.table.slots_len()
        };

        let min_free_fd = get_min_free_fd();
        let entry = FileTableEntry::new(file, flags);
        self.table.put_at(min_free_fd, entry);
        Ok(min_free_fd as FileDescripter)
    }

    pub fn insert(&mut self, item: Arc<dyn FileLike>, flags: FdFlags) -> FileDescripter {
        let entry = FileTableEntry::new(item, flags);
        self.table.put(entry) as FileDescripter
    }

    pub fn insert_at(
        &mut self,
        fd: FileDescripter,
        item: Arc<dyn FileLike>,
        flags: FdFlags,
    ) -> Option<Arc<dyn FileLike>> {
        let entry = FileTableEntry::new(item, FdFlags::empty());
        let entry = self.table.put_at(fd as usize, entry);
        if entry.is_some() {
            let events = FdEvents::Close(fd);
            self.notify_fd_events(&events);
            entry.as_ref().unwrap().notify_fd_events(&events);
        }
        entry.map(|e| e.file)
    }

    pub fn close_file(&mut self, fd: FileDescripter) -> Option<Arc<dyn FileLike>> {
        let entry = self.table.remove(fd as usize);
        if entry.is_some() {
            let events = FdEvents::Close(fd);
            self.notify_fd_events(&events);
            entry.as_ref().unwrap().notify_fd_events(&events);
        }
        entry.map(|e| e.file)
    }

    pub fn close_all(&mut self) -> Vec<Arc<dyn FileLike>> {
        let mut closed_files = Vec::new();
        let closed_fds: Vec<FileDescripter> = self
            .table
            .idxes_and_items()
            .map(|(idx, _)| idx as FileDescripter)
            .collect();
        for fd in closed_fds {
            let entry = self.table.remove(fd as usize).unwrap();
            let events = FdEvents::Close(fd);
            self.notify_fd_events(&events);
            entry.notify_fd_events(&events);
            closed_files.push(entry.file);
        }
        closed_files
    }

    pub fn close_files_on_exec(&mut self) -> Vec<Arc<dyn FileLike>> {
        let mut closed_files = Vec::new();
        let closed_fds: Vec<FileDescripter> = self
            .table
            .idxes_and_items()
            .filter_map(|(idx, entry)| {
                if entry.flags().contains(FdFlags::CLOEXEC) {
                    Some(idx as FileDescripter)
                } else {
                    None
                }
            })
            .collect();
        for fd in closed_fds {
            let entry = self.table.remove(fd as usize).unwrap();
            let events = FdEvents::Close(fd);
            self.notify_fd_events(&events);
            entry.notify_fd_events(&events);
            closed_files.push(entry.file);
        }
        closed_files
    }

    pub fn get_file(&self, fd: FileDescripter) -> Result<&Arc<dyn FileLike>> {
        self.table
            .get(fd as usize)
            .map(|entry| &entry.file)
            .ok_or(Error::with_message(Errno::EBADF, "fd not exits"))
    }

    pub fn get_socket(&self, sockfd: FileDescripter) -> Result<Arc<dyn Socket>> {
        let file_like = self.get_file(sockfd)?.clone();
        file_like
            .as_socket()
            .ok_or_else(|| Error::with_message(Errno::ENOTSOCK, "the fd is not a socket"))
    }

    pub fn get_entry(&self, fd: FileDescripter) -> Result<&FileTableEntry> {
        self.table
            .get(fd as usize)
            .ok_or(Error::with_message(Errno::EBADF, "fd not exits"))
    }

    pub fn fds_and_files(&self) -> impl Iterator<Item = (FileDescripter, &'_ Arc<dyn FileLike>)> {
        self.table
            .idxes_and_items()
            .map(|(idx, entry)| (idx as FileDescripter, &entry.file))
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

#[derive(Copy, Clone)]
pub enum FdEvents {
    Close(FileDescripter),
    DropFileTable,
}

impl Events for FdEvents {}

pub struct FileTableEntry {
    file: Arc<dyn FileLike>,
    flags: AtomicU8,
    subject: Subject<FdEvents>,
}

impl FileTableEntry {
    pub fn new(file: Arc<dyn FileLike>, flags: FdFlags) -> Self {
        Self {
            file,
            flags: AtomicU8::new(flags.bits()),
            subject: Subject::new(),
        }
    }

    pub fn file(&self) -> &Arc<dyn FileLike> {
        &self.file
    }

    pub fn flags(&self) -> FdFlags {
        FdFlags::from_bits(self.flags.load(Ordering::Relaxed)).unwrap()
    }

    pub fn set_flags(&self, flags: FdFlags) {
        self.flags.store(flags.bits(), Ordering::Relaxed);
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
        }
    }
}

bitflags! {
    pub struct FdFlags: u8 {
        /// Close on exec
        const CLOEXEC = 1;
    }
}
