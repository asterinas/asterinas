// SPDX-License-Identifier: MPL-2.0

#![expect(unused_variables)]

//! Opened File Handle

use core::fmt::Display;

use ostd::io::IoMem;

use super::{
    AccessMode, FileCommon, InodeHandle, SettableStatusFlags, StatusFlags, file_table::FdFlags,
    inode_handle::SeekFrom,
};
use crate::{
    fs::vfs::{inode::FallocMode, path::Path},
    net::socket::Socket,
    prelude::*,
    process::{Process, signal::Pollable},
    util::ioctl::RawIoctl,
    vm::page_cache::Vmo,
};

/// The basic operations defined on a file
pub trait FileLike: Pollable + Send + Sync + Any {
    /// Reads data from this file.
    ///
    /// By default, this method returns `EBADF`
    /// if the file is not readable,
    /// or `EINVAL` if the file type does not support `read`.
    /// These are distinct conditions:
    /// [`access_mode`] may say that a file is readable,
    /// while the file type still does not support `read`,
    /// as with epoll files and pid files.
    ///
    /// Implementors should override this method if the file type supports reads.
    /// An overriding implementation must check whether [`access_mode`] is readable
    /// before performing the operation.
    ///
    /// [`access_mode`]: FileLike::access_mode
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        if !self.access_mode().is_readable() {
            return_errno_with_message!(Errno::EBADF, "the file is not opened for reading");
        }
        return_errno_with_message!(Errno::EINVAL, "read is not supported for this file type");
    }

    /// Writes data to this file.
    ///
    /// By default, this method returns `EBADF`
    /// if the file is not writable,
    /// or `EINVAL` if the file type does not support `write`.
    /// These are distinct conditions:
    /// [`access_mode`] may say that a file is writable,
    /// while the file type still does not support `write`,
    /// as with epoll files and pid files.
    ///
    /// Implementors should override this method if the file type supports writes.
    /// An overriding implementation must check whether [`access_mode`] is writable
    /// before performing the operation.
    ///
    /// [`access_mode`]: FileLike::access_mode
    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        if !self.access_mode().is_writable() {
            return_errno_with_message!(Errno::EBADF, "the file is not opened for writing");
        }
        return_errno_with_message!(Errno::EINVAL, "write is not supported for this file type");
    }

    /// Read at the given file offset.
    ///
    /// The file must be seekable to support `read_at`.
    /// Unlike [`read`], `read_at` will not change the file offset.
    ///
    /// [`read`]: FileLike::read
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::ESPIPE, "read_at is not supported");
    }

    /// Write at the given file offset.
    ///
    /// The file must be seekable to support `write_at`.
    /// Unlike [`write`], `write_at` will not change the file offset.
    /// If the file is append-only, the `offset` will be ignored.
    ///
    /// [`write`]: FileLike::write
    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::ESPIPE, "write_at is not supported");
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        // `ENOTTY` means that "The specified operation does not apply to the kind of object that
        // the file descriptor references".
        // Reference: <https://man7.org/linux/man-pages/man2/ioctl.2.html>.
        return_errno_with_message!(Errno::ENOTTY, "ioctl is not supported");
    }

    /// Obtains the mappable object to map this file into the user address space.
    ///
    /// If this file has a corresponding mappable object of [`Mappable`],
    /// then it can be either an inode or an MMIO region.
    fn mappable(&self) -> Result<Mappable> {
        // `ENODEV` means that "The underlying filesystem of the specified file does not support
        // memory mapping".
        // Reference: <https://man7.org/linux/man-pages/man2/mmap.2.html>.
        return_errno_with_message!(Errno::ENODEV, "the file is not mappable");
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "resize is not supported");
    }

    fn status_flags(&self) -> StatusFlags {
        self.common().status_flags()
    }

    /// Returns the status flags that can be set for this file.
    fn settable_status_flags(&self) -> SettableStatusFlags {
        // `O_ASYNC` and `O_DIRECT` can only be set on file descriptions that explicitly
        // support them.
        SettableStatusFlags::minimal()
    }

    /// Returns the access mode of this file.
    ///
    /// The access mode indicates whether the file descriptor is readable or writable.
    /// Readability is required for operations
    /// such as [`read`], [`read_at`], and read-only memory mappings.
    /// Writability is required for operations
    /// such as [`write`], [`write_at`], [`resize`], [`fallocate`],
    /// and writable memory mappings.
    ///
    /// For inode-backed files,
    /// their access modes are determined dynamically for each opened file.
    /// For special files such as epoll files, eventfd files, and pid files,
    /// they are determined statically by the file types.
    ///
    /// [`read`]: FileLike::read
    /// [`read_at`]: FileLike::read_at
    /// [`write`]: FileLike::write
    /// [`write_at`]: FileLike::write_at
    /// [`resize`]: FileLike::resize
    /// [`fallocate`]: FileLike::fallocate
    fn access_mode(&self) -> AccessMode;

    fn seek(&self, seek_from: SeekFrom) -> Result<usize> {
        return_errno_with_message!(Errno::ESPIPE, "seek is not supported");
    }

    fn fallocate(&self, _mode: FallocMode, _offset: usize, _len: usize) -> Result<()> {
        return_errno_with_message!(
            Errno::ENODEV,
            "fallocate is not supported for this file type"
        );
    }

    fn as_socket(&self) -> Option<&dyn Socket> {
        None
    }

    /// Returns the common state shared by file-like objects.
    fn common(&self) -> &FileCommon;

    /// Dumps information to appear in the `fdinfo` file under procfs.
    ///
    /// This method must not break atomic mode because it will be called with the file table's spin
    /// lock held. There are two strategies for implementing this method:
    ///  - If the necessary information can be obtained without breaking atomic mode, the method
    ///    can collect and return the information directly. `Arc<Self>` should be dropped and
    ///    should not appear in the returned `Box<dyn Display>`.
    ///  - Otherwise, if the file can be dropped asynchronously in another process, the method can
    ///    return a `Box<dyn Display>` containing the `Arc<Self>`, so that the information can be
    ///    collected later in its `Display::display()` method, after dropping the file table's spin
    ///    lock.
    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display>;
}

impl dyn FileLike {
    /// Returns the path associated with the file description.
    pub fn path(&self) -> &Path {
        self.common().path()
    }

    /// Updates file status flags atomically.
    ///
    /// `O_ASYNC` is ignored if it is not supported. An attempt to enable
    /// unsupported `O_DIRECT` returns `EINVAL`.
    pub fn update_status_flags(&self, mut update: StatusFlagsUpdate) -> Result<()> {
        let settable_flags = self.settable_status_flags();
        if update.flags().contains(StatusFlags::O_DIRECT)
            && !settable_flags.contains(StatusFlags::O_DIRECT)
        {
            return_errno_with_message!(Errno::EINVAL, "the `O_DIRECT` flag is not supported");
        }
        if !settable_flags.contains(StatusFlags::O_ASYNC) {
            update.ignore(StatusFlags::O_ASYNC);
        }

        self.common().update_status_flags(self, update);
        Ok(())
    }

    /// Updates the `O_NONBLOCK` status flag.
    pub fn update_status_nonblock(&self, is_nonblocking: bool) {
        let update = if is_nonblocking {
            StatusFlagsUpdate::set(StatusFlags::O_NONBLOCK)
        } else {
            StatusFlagsUpdate::unset(StatusFlags::O_NONBLOCK)
        };

        self.common().update_status_flags(self, update);
    }

    /// Updates the `O_ASYNC` status flag.
    ///
    /// An attempt to enable `O_ASYNC` on a file that does not support it returns
    /// `ENOTTY`.
    pub fn update_status_async(&self, is_async: bool) -> Result<()> {
        let settable_flags = self.settable_status_flags();
        if is_async && !settable_flags.contains(StatusFlags::O_ASYNC) {
            return_errno_with_message!(Errno::ENOTTY, "signal-driven I/O is not supported");
        }

        let update = if is_async {
            StatusFlagsUpdate::set(StatusFlags::O_ASYNC)
        } else {
            StatusFlagsUpdate::unset(StatusFlags::O_ASYNC)
        };

        self.common().update_status_flags(self, update);
        Ok(())
    }

    /// Sets a process as the owner of the file description.
    ///
    /// Passing `None` clears the current owner.
    ///
    /// The owner receives `SIGIO` for I/O events on the file description when `O_ASYNC` is set.
    pub fn set_owner(&self, owner: Option<&Arc<Process>>) {
        self.common().owner().set(self, owner);
    }

    pub fn downcast_ref<T: FileLike>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }

    pub fn read_bytes(&self, buf: &mut [u8]) -> Result<usize> {
        let mut writer = VmWriter::from(buf).to_fallible();
        self.read(&mut writer)
    }

    pub fn write_bytes(&self, buf: &[u8]) -> Result<usize> {
        let mut reader = VmReader::from(buf).to_fallible();
        self.write(&mut reader)
    }

    pub fn read_bytes_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let mut writer = VmWriter::from(buf).to_fallible();
        self.read_at(offset, &mut writer)
    }

    #[expect(dead_code)]
    pub fn write_bytes_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let mut reader = VmReader::from(buf).to_fallible();
        self.write_at(offset, &mut reader)
    }

    pub fn as_socket_or_err(&self) -> Result<&dyn Socket> {
        self.as_socket()
            .ok_or_else(|| Error::with_message(Errno::ENOTSOCK, "the file is not a socket"))
    }

    pub fn as_inode_handle_or_err(&self) -> Result<&InodeHandle> {
        self.downcast_ref().ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "the file is not related to an inode")
        })
    }
}

/// An atomic update to a subset of file status flags.
#[derive(Clone, Copy, Debug)]
pub struct StatusFlagsUpdate {
    mask: StatusFlags,
    flags: StatusFlags,
}

impl StatusFlagsUpdate {
    /// Creates an update that replaces all updatable flags with `flags`.
    ///
    /// Replacing flags outside [`StatusFlags::SETFL_MASK`] is a no-op.
    pub fn replace(mut flags: StatusFlags) -> Self {
        flags &= StatusFlags::SETFL_MASK;
        Self::new(StatusFlags::SETFL_MASK, flags)
    }

    /// Creates an update that sets `flags`.
    ///
    /// Setting flags outside [`StatusFlags::SETFL_MASK`] is a no-op.
    pub fn set(mut flags: StatusFlags) -> Self {
        flags &= StatusFlags::SETFL_MASK;
        Self::new(flags, flags)
    }

    /// Creates an update that unsets `flags`.
    ///
    /// Unsetting flags outside [`StatusFlags::SETFL_MASK`] is a no-op.
    pub fn unset(mut flags: StatusFlags) -> Self {
        flags &= StatusFlags::SETFL_MASK;
        Self::new(flags, StatusFlags::empty())
    }

    /// Makes this update leave `flags` unchanged.
    fn ignore(&mut self, flags: StatusFlags) {
        self.mask.remove(flags);
        self.flags &= self.mask;
    }

    fn new(mask: StatusFlags, flags: StatusFlags) -> Self {
        Self { mask, flags }
    }

    /// Returns the flags that this update will set.
    pub(super) fn flags(self) -> StatusFlags {
        self.flags
    }

    /// Returns whether this update affects any of the specified flags.
    pub(super) fn affects(self, flags: StatusFlags) -> bool {
        self.mask.intersects(flags)
    }

    /// Applies this update to the current file status flags.
    pub(super) fn apply(self, current: StatusFlags) -> StatusFlags {
        (current - self.mask) | self.flags
    }
}

/// An object that may be memory mapped into the user address space.
#[derive(Clone, Debug)]
pub enum Mappable {
    /// A VMO (i.e., page cache).
    Vmo(Arc<Vmo>),
    /// An MMIO region.
    IoMem(IoMem),
}
