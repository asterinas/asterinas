// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use core::sync::atomic::{AtomicU8, Ordering};

use aster_util::slot_vec::SlotVec;

use super::{
    file_handle::FileLike,
    fs_resolver::{FsPath, FsResolver, AT_FDCWD},
    utils::{AccessMode, InodeMode},
};
use crate::{
    events::{Events, IoEvents, Observer, Subject},
    fs::utils::StatusFlags,
    net::socket::Socket,
    prelude::*,
    process::{
        process_table,
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

    pub fn dup(&mut self, fd: FileDesc, new_fd: FileDesc, flags: FdFlags) -> Result<FileDesc> {
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
        entry.map(|e| e.file)
    }

    pub fn close_file(&mut self, fd: FileDesc) -> Option<Arc<dyn FileLike>> {
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
        let closed_fds: Vec<FileDesc> = self
            .table
            .idxes_and_items()
            .map(|(idx, _)| idx as FileDesc)
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
        let closed_fds: Vec<FileDesc> = self
            .table
            .idxes_and_items()
            .filter_map(|(idx, entry)| {
                if entry.flags().contains(FdFlags::CLOEXEC) {
                    Some(idx as FileDesc)
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

    pub fn get_file(&self, fd: FileDesc) -> Result<&Arc<dyn FileLike>> {
        self.table
            .get(fd as usize)
            .map(|entry| &entry.file)
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

    pub fn fds_and_files(&self) -> impl Iterator<Item = (FileDesc, &'_ Arc<dyn FileLike>)> {
        self.table
            .idxes_and_items()
            .map(|(idx, entry)| (idx as FileDesc, &entry.file))
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

#[derive(Copy, Clone)]
pub enum FdEvents {
    Close(FileDesc),
    DropFileTable,
}

impl Events for FdEvents {}

pub struct FileTableEntry {
    file: Arc<dyn FileLike>,
    flags: AtomicU8,
    subject: Subject<FdEvents>,
    owner: RwLock<Option<Owner>>,
}

impl FileTableEntry {
    pub fn new(file: Arc<dyn FileLike>, flags: FdFlags) -> Self {
        Self {
            file,
            flags: AtomicU8::new(flags.bits()),
            subject: Subject::new(),
            owner: RwLock::new(None),
        }
    }

    pub fn file(&self) -> &Arc<dyn FileLike> {
        &self.file
    }

    pub fn owner(&self) -> Option<Pid> {
        self.owner.read().as_ref().map(|(pid, _)| *pid)
    }

    /// Set a process (group) as owner of the file descriptor.
    ///
    /// Such that this process (group) will receive `SIGIO` and `SIGURG` signals
    /// for I/O events on the file descriptor, if `O_ASYNC` status flag is set
    /// on this file.
    pub fn set_owner(&self, owner: Pid) -> Result<()> {
        if self.owner().is_some_and(|pid| pid == owner) {
            return Ok(());
        }

        // Unset the owner if the given pid is zero.
        let new_owner = if owner == 0 {
            None
        } else {
            let process = process_table::get_process(owner as _).ok_or(Error::with_message(
                Errno::ESRCH,
                "cannot set_owner with an invalid pid",
            ))?;
            let observer = OwnerObserver::new(self.file.clone(), Arc::downgrade(&process));
            Some((owner, observer))
        };

        let mut self_owner = self.owner.write();
        if let Some((_, observer)) = self_owner.as_ref() {
            let _ = self.file.unregister_observer(&Arc::downgrade(observer));
        }

        *self_owner = match new_owner {
            None => None,
            Some((pid, observer)) => {
                self.file
                    .register_observer(observer.weak_self(), IoEvents::empty())?;
                Some((pid, observer))
            }
        };
        Ok(())
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
            owner: RwLock::new(self.owner.read().clone()),
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
