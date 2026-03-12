// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU8, Ordering};

use aster_util::{ranged_integer::RangedU32, slot_vec::SlotVec};

use super::{StatusFlags, file_handle::FileLike};
use crate::{
    events::{IoEvents, Observer},
    prelude::*,
    process::{
        Pid, Process,
        posix_thread::FileTableRefMut,
        signal::{PollAdaptor, constants::SIGIO},
    },
};

pub type FileDesc = RangedU32<0, { i32::MAX as _ }>;
pub type RawFileDesc = i32;

#[derive(Clone)]
pub struct FileTable {
    table: SlotVec<FileTableEntry>,
}

impl FileTable {
    pub const fn new() -> Self {
        Self {
            table: SlotVec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.table.slots_len()
    }

    /// Duplicates `fd` onto the lowest-numbered available descriptor equal to
    /// or greater than `ceil_fd`.
    pub fn dup_ceil(
        &mut self,
        fd: FileDesc,
        ceil_fd: FileDesc,
        flags: FdFlags,
    ) -> Result<FileDesc> {
        let entry = self.duplicate_entry(fd, flags)?;

        // Get the lowest-numbered available fd equal to or greater than `ceil_fd`.
        let get_min_free_fd = || -> usize {
            let ceil_fd = ceil_fd.get() as _;
            if self.table.get(ceil_fd).is_none() {
                return ceil_fd;
            }

            for idx in ceil_fd + 1..self.len() {
                if self.table.get(idx).is_none() {
                    return idx;
                }
            }
            self.len()
        };

        let min_free_fd = get_min_free_fd();
        self.table.put_at(min_free_fd, entry);
        Ok(FileDesc::new(min_free_fd as _))
    }

    /// Duplicates `fd` onto the exact descriptor number `new_fd`.
    pub fn dup_exact(
        &mut self,
        fd: FileDesc,
        new_fd: FileDesc,
        flags: FdFlags,
    ) -> Result<Option<Arc<dyn FileLike>>> {
        let entry = self.duplicate_entry(fd, flags)?;
        let closed_file = self.close_file(new_fd);
        self.table.put_at(new_fd.get() as _, entry);
        Ok(closed_file)
    }

    fn duplicate_entry(&self, fd: FileDesc, flags: FdFlags) -> Result<FileTableEntry> {
        let file = self
            .table
            .get(fd.get() as _)
            .map(|entry| entry.file.clone())
            .ok_or(Error::with_message(Errno::EBADF, "fd does not exist"))?;
        Ok(FileTableEntry::new(file, flags))
    }

    pub fn insert(&mut self, item: Arc<dyn FileLike>, flags: FdFlags) -> FileDesc {
        let entry = FileTableEntry::new(item, flags);
        FileDesc::new(self.table.put(entry) as _)
    }

    pub fn close_file(&mut self, fd: FileDesc) -> Option<Arc<dyn FileLike>> {
        let removed_entry = self.table.remove(fd.get() as _)?;
        // POSIX record locks are process-associated and Linux drops them when any fd for the inode is
        // closed by that process, even if duplicated descriptors still exist.
        //
        // Reference: <https://man7.org/linux/man-pages/man2/fcntl_locking.2.html>
        if let Ok(inode_handle) = removed_entry.file.as_inode_handle_or_err() {
            inode_handle.release_range_locks();
        }
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
                    Some(FileDesc::new(idx as _))
                } else {
                    None
                }
            })
            .collect();

        for fd in closed_fds {
            closed_files.push(self.close_file(fd).unwrap());
        }

        closed_files
    }

    pub fn get_file(&self, fd: FileDesc) -> Result<&Arc<dyn FileLike>> {
        self.table
            .get(fd.get() as _)
            .map(|entry| entry.file())
            .ok_or(Error::with_message(Errno::EBADF, "fd not exits"))
    }

    pub fn get_entry(&self, fd: FileDesc) -> Result<&FileTableEntry> {
        self.table
            .get(fd.get() as _)
            .ok_or(Error::with_message(Errno::EBADF, "fd not exits"))
    }

    pub fn get_entry_mut(&mut self, fd: FileDesc) -> Result<&mut FileTableEntry> {
        self.table
            .get_mut(fd.get() as _)
            .ok_or(Error::with_message(Errno::EBADF, "fd not exits"))
    }

    pub fn fds_and_files(&self) -> impl Iterator<Item = (FileDesc, &'_ Arc<dyn FileLike>)> {
        self.table
            .idxes_and_items()
            .map(|(idx, entry)| (FileDesc::new(idx as _), entry.file()))
    }
}

impl Default for FileTable {
    fn default() -> Self {
        Self::new()
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
/// [`RwArc::read`]: ostd::sync::RwArc::read
macro_rules! get_file_fast {
    ($file_table:expr, $file_desc:expr) => {{
        use alloc::borrow::Cow;

        use ostd::sync::RwArc;
        use $crate::{
            fs::file::file_table::{FileDesc, FileTable},
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

pub struct FileTableEntry {
    file: Arc<dyn FileLike>,
    flags: AtomicU8,
    owner: Option<Owner>,
}

impl FileTableEntry {
    pub fn new(file: Arc<dyn FileLike>, flags: FdFlags) -> Self {
        Self {
            file,
            flags: AtomicU8::new(flags.bits()),
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
}

impl Clone for FileTableEntry {
    fn clone(&self) -> Self {
        Self {
            file: self.file.clone(),
            flags: AtomicU8::new(self.flags.load(Ordering::Relaxed)),
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
