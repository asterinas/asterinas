// SPDX-License-Identifier: MPL-2.0

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
    prelude::*,
    process::{
        posix_thread::FileTableRefMut,
        signal::{constants::SIGIO, PollAdaptor},
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
            .map(|entry| entry.file.clone())
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
        entry.map(|e| e.file)
    }

    pub fn close_file(&mut self, fd: FileDesc) -> Option<Arc<dyn FileLike>> {
        let removed_entry = self.table.remove(fd as usize)?;

        let events = FdEvents::Close(fd);
        self.notify_fd_events(&events);
        removed_entry.notify_fd_events(&events);

        Some(removed_entry.file)
    }

    pub fn close_files_on_exec(&mut self) -> Vec<Arc<dyn FileLike>> {
        self.close_files(|entry| entry.flags().contains(FdFlags::CLOEXEC))
    }

    fn close_files<F>(&mut self, should_close: F) -> Vec<Arc<dyn FileLike>>
    where
        F: Fn(&FileTableEntry) -> bool,
    {
        let mut closed_files = Vec::new();
        let closed_fds: Vec<FileDesc> = self
            .table
            .idxes_and_items()
            .filter_map(|(idx, entry)| {
                if should_close(entry) {
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
            closed_files.push(removed_entry.file);
        }

        closed_files
    }

    pub fn get_file(&self, fd: FileDesc) -> Result<&Arc<dyn FileLike>> {
        self.table
            .get(fd as usize)
            .map(|entry| &entry.file)
            .ok_or(Error::with_message(Errno::EBADF, "fd not exits"))
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
        // Closes all files first.
        self.close_files(|_| true);

        let events = FdEvents::DropFileTable;
        self.subject.notify_observers(&events);
    }
}

/// A helper trait that provides methods to operate the file table.
pub trait WithFileTable {
    /// Calls `f` with the file table.
    ///
    /// This method is lockless if the file table is not shared. Otherwise, `f` is called while
    /// holding the read lock on the file table.
    fn read_with<R>(&mut self, f: impl FnOnce(&FileTable) -> R) -> R;
}

impl WithFileTable for FileTableRefMut<'_> {
    fn read_with<R>(&mut self, f: impl FnOnce(&FileTable) -> R) -> R {
        let file_table = self.unwrap();

        if let Some(inner) = file_table.get() {
            f(inner)
        } else {
            f(&file_table.read())
        }
    }
}

/// Gets a file from a file descriptor as fast as possible.
///
/// `file_table` should be a mutable borrow of the file table contained in the `file_table` field
/// (which is a [`RefCell`]) in [`ThreadLocal`]. A mutable borrow is required because its
/// exclusivity can be useful for achieving lockless file lookups.
///
/// If the file table is not shared with another thread, this macro will be free of locks
/// ([`RwArc::read`]) and free of reference counting ([`Arc::clone`]).
///
/// If the file table is shared, the read lock is taken, the file is cloned, and then the read lock
/// is released. Cloning and releasing the lock is necessary because we cannot hold such locks when
/// operating on files, since many operations on files can block.
///
/// Note: This has to be a macro due to a limitation in the Rust borrow check implementation. Once
/// <https://github.com/rust-lang/rust/issues/58910> is fixed, we can try to convert this macro to
/// a function.
///
/// [`RefCell`]: core::cell::RefCell
/// [`ThreadLocal`]: crate::process::posix_thread::ThreadLocal
macro_rules! get_file_fast {
    ($file_table:expr, $file_desc:expr) => {{
        use alloc::borrow::Cow;

        use ostd::sync::RwArc;
        use $crate::{
            fs::file_table::{FileDesc, FileTable},
            process::posix_thread::FileTableRefMut,
        };

        let file_table: &mut FileTableRefMut<'_> = $file_table;
        let file_table: &mut RwArc<FileTable> = file_table.unwrap();
        let file_desc: FileDesc = $file_desc;

        if let Some(inner) = file_table.get() {
            // Fast path: The file table is not shared, we can get the file in a lockless way.
            Cow::Borrowed(inner.get_file(file_desc)?)
        } else {
            // Slow path: The file table is shared, we need to hold the lock and clone the file.
            Cow::Owned(file_table.read().get_file(file_desc)?.clone())
        }
    }};
}

pub(crate) use get_file_fast;

#[derive(Copy, Clone, Debug)]
pub enum FdEvents {
    Close(FileDesc),
    DropFileTable,
}

impl Events for FdEvents {}

pub struct FileTableEntry {
    file: Arc<dyn FileLike>,
    flags: AtomicU8,
    subject: Subject<FdEvents>,
    owner: Option<Owner>,
}

impl FileTableEntry {
    pub fn new(file: Arc<dyn FileLike>, flags: FdFlags) -> Self {
        Self {
            file,
            flags: AtomicU8::new(flags.bits()),
            subject: Subject::new(),
            owner: None,
        }
    }

    pub fn file(&self) -> &Arc<dyn FileLike> {
        &self.file
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
        let Some(process) = owner else {
            self.owner = None;
            return Ok(());
        };

        let mut poller = PollAdaptor::with_observer(OwnerObserver::new(
            self.file.clone(),
            Arc::downgrade(process),
        ));
        self.file
            .poll(IoEvents::IN | IoEvents::OUT, Some(poller.as_handle_mut()));

        self.owner = Some((process.pid(), poller));

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
            owner: None,
        }
    }
}

bitflags! {
    pub struct FdFlags: u8 {
        /// Close on exec
        const CLOEXEC = 1;
    }
}

type Owner = (Pid, PollAdaptor<OwnerObserver>);

struct OwnerObserver {
    file: Arc<dyn FileLike>,
    owner: Weak<Process>,
}

impl OwnerObserver {
    pub fn new(file: Arc<dyn FileLike>, owner: Weak<Process>) -> Self {
        Self { file, owner }
    }
}

impl Observer<IoEvents> for OwnerObserver {
    fn on_events(&self, _events: &IoEvents) {
        if self.file.status_flags().contains(StatusFlags::O_ASYNC) {
            crate::process::enqueue_signal_async(self.owner.clone(), SIGIO);
        }
    }
}
