// SPDX-License-Identifier: MPL-2.0

use core::{
    fmt::Display,
    sync::atomic::{AtomicU8, Ordering},
};

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

/// Represents a validated, non-negative file descriptor.
///
/// The value is guaranteed to be in the range `[0, i32::MAX]`.
/// Use [`RawFileDesc`] at syscall boundaries,
/// then convert to `FileDesc` via `TryFrom` for kernel-internal use.
///
/// Some system calls (e.g., `fcntl`) reinterpret
/// values of types other than [`RawFileDesc`] (e.g., `u64` or `usize`)
/// as file descriptors. Linux typically truncates the high bits
/// without checking whether the full argument fits in range.
/// To avoid accidental misuse, we do not implement `TryFrom` for
/// those types. The syscall layer should first convert them to
/// a [`RawFileDesc`] in an explicit, syscall-specific way,
/// and then convert that value to a `FileDesc`.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FileDesc(RangedU32<0, { i32::MAX as _ }>);

/// A raw file descriptor as received from or returned to user space.
///
/// This is the `int` type from Linux syscall signatures.
/// It may hold negative sentinel values like `AT_FDCWD` (-100).
/// Convert to [`FileDesc`] via `TryFrom` before use.
pub type RawFileDesc = i32;

impl FileDesc {
    /// File descriptor 0.
    pub const ZERO: Self = Self(RangedU32::new(0));
}

impl Display for FileDesc {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0.get())
    }
}

impl From<FileDesc> for RawFileDesc {
    fn from(value: FileDesc) -> Self {
        value.0.get() as _
    }
}

impl From<FileDesc> for isize {
    fn from(value: FileDesc) -> Self {
        value.0.get() as _
    }
}

impl From<FileDesc> for u32 {
    fn from(value: FileDesc) -> Self {
        value.0.get()
    }
}

impl From<FileDesc> for u64 {
    fn from(value: FileDesc) -> Self {
        value.0.get() as _
    }
}

impl From<FileDesc> for usize {
    fn from(value: FileDesc) -> Self {
        value.0.get() as _
    }
}

// Intentionally, `TryFrom<RawFileDesc>` is
// the only `TryFrom` implementation for `FileDesc`.
//
// We do not implement conversions from wider integer types
// for Linux compatibility reasons; see the `FileDesc` type docs.
// We also do not implement `TryFrom<u32>` directly,
// to encourage callers to name `RawFileDesc` explicitly
// instead of using a plain `u32`.
impl TryFrom<RawFileDesc> for FileDesc {
    type Error = Error;

    fn try_from(value: RawFileDesc) -> Result<Self> {
        if value < 0 {
            return_errno_with_message!(Errno::EBADF, "negative FDs are not valid");
        }
        Ok(Self(RangedU32::new(value.cast_unsigned())))
    }
}

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
            let ceil_fd = ceil_fd.into();
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
        // Resource limits guarantee the table never exceeds `i32::MAX` entries.
        Ok((min_free_fd as RawFileDesc).try_into().unwrap())
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
        self.table.put_at(new_fd.into(), entry);
        Ok(closed_file)
    }

    fn duplicate_entry(&self, fd: FileDesc, flags: FdFlags) -> Result<FileTableEntry> {
        let file = self
            .table
            .get(fd.into())
            .map(|entry| entry.file.clone())
            .ok_or(Error::with_message(Errno::EBADF, "the FD does not exist"))?;
        Ok(FileTableEntry::new(file, flags))
    }

    pub fn insert(&mut self, item: Arc<dyn FileLike>, flags: FdFlags) -> FileDesc {
        let entry = FileTableEntry::new(item, flags);
        // Resource limits guarantee the table never exceeds `i32::MAX` entries.
        (self.table.put(entry) as RawFileDesc).try_into().unwrap()
    }

    pub fn close_file(&mut self, fd: FileDesc) -> Option<Arc<dyn FileLike>> {
        let removed_entry = self.table.remove(fd.into())?;
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
                    // Resource limits guarantee the table never exceeds `i32::MAX` entries.
                    Some((idx as RawFileDesc).try_into().unwrap())
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
            .get(fd.into())
            .map(|entry| entry.file())
            .ok_or(Error::with_message(Errno::EBADF, "the FD does not exist"))
    }

    pub fn get_entry(&self, fd: FileDesc) -> Result<&FileTableEntry> {
        self.table
            .get(fd.into())
            .ok_or(Error::with_message(Errno::EBADF, "the FD does not exist"))
    }

    pub fn get_entry_mut(&mut self, fd: FileDesc) -> Result<&mut FileTableEntry> {
        self.table
            .get_mut(fd.into())
            .ok_or(Error::with_message(Errno::EBADF, "the FD does not exist"))
    }

    pub fn fds_and_files(&self) -> impl Iterator<Item = (FileDesc, &'_ Arc<dyn FileLike>)> {
        // Resource limits guarantee the table never exceeds `i32::MAX` entries.
        self.table
            .idxes_and_items()
            .map(|(idx, entry)| ((idx as RawFileDesc).try_into().unwrap(), entry.file()))
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
