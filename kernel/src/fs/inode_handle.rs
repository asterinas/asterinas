// SPDX-License-Identifier: MPL-2.0

//! Opened Inode-backed File Handle

use core::{fmt::Display, sync::atomic::Ordering};

use aster_rights::Rights;

use super::utils::{InodeExt, InodeIo};
use crate::{
    events::IoEvents,
    fs::{
        file_handle::{FileLike, Mappable},
        file_table::FdFlags,
        path::Path,
        pipe::PipeHandle,
        utils::{
            AccessMode, AtomicStatusFlags, CreationFlags, DirentVisitor, FallocMode, FileRange,
            FlockItem, InodeType, OFFSET_MAX, RangeLockItem, RangeLockType, SeekFrom, StatusFlags,
        },
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
};

pub struct InodeHandle {
    path: Path,
    /// `file_io` is similar to the `file_private` field in Linux's `file` structure. If `file_io`
    /// is `Some(_)`, typical file operations including `read`, `write`, `poll`, and `ioctl` will
    /// be provided by `file_io`, instead of `path`.
    file_io: Option<Box<dyn FileIo>>,
    offset: Mutex<usize>,
    status_flags: AtomicStatusFlags,
    rights: Rights,
}

impl InodeHandle {
    pub fn new(path: Path, access_mode: AccessMode, status_flags: StatusFlags) -> Result<Self> {
        let inode = path.inode();
        if !status_flags.contains(StatusFlags::O_PATH) {
            // "Opening a file or directory with the O_PATH flag requires no permissions on the
            // object itself".
            // Reference: <https://man7.org/linux/man-pages/man2/openat.2.html>
            inode.check_permission(access_mode.into())?;
        }

        Self::new_unchecked_access(path, access_mode, status_flags)
    }

    pub fn new_unchecked_access(
        path: Path,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Result<Self> {
        let inode = path.inode();
        let (file_io, rights) = if status_flags.contains(StatusFlags::O_PATH) {
            (None, Rights::empty())
        } else if inode.type_() == InodeType::Dir && access_mode.is_writable() {
            return_errno_with_message!(Errno::EISDIR, "a directory cannot be opened writable");
        } else {
            let file_io = inode.open(access_mode, status_flags).transpose()?;
            let rights = Rights::from(access_mode);
            (file_io, rights)
        };

        Ok(Self {
            path,
            file_io,
            offset: Mutex::new(0),
            status_flags: AtomicStatusFlags::new(status_flags),
            rights,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn offset(&self) -> usize {
        let offset = self.offset.lock();
        *offset
    }

    pub(super) fn rights(&self) -> Rights {
        self.rights
    }

    fn inode_io_and_is_offset_aware(&self) -> (&dyn InodeIo, bool) {
        if let Some(ref file_io) = self.file_io {
            let is_offset_aware = file_io.is_offset_aware();
            return (file_io.as_ref(), is_offset_aware);
        }

        let inode = self.path.inode();
        let is_offset_aware = inode.type_().is_seekable();
        (inode.as_ref(), is_offset_aware)
    }

    fn inode_io_and_check_seekable(&self) -> Result<&dyn InodeIo> {
        if let Some(ref file_io) = self.file_io {
            file_io.check_seekable()?;
            return Ok(file_io.as_ref());
        }

        let inode = self.path.inode();
        if !inode.type_().is_seekable() {
            return_errno_with_message!(
                Errno::ESPIPE,
                "the inode cannot be read or written at a specific offset"
            );
        }
        Ok(inode.as_ref())
    }

    pub fn readdir(&self, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        if !self.rights.contains(Rights::READ) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened readable");
        }

        let mut offset = self.offset.lock();
        let read_cnt = self.path.inode().readdir_at(*offset, visitor)?;
        *offset += read_cnt;
        Ok(read_cnt)
    }

    pub fn test_range_lock(&self, mut lock: RangeLockItem) -> Result<RangeLockItem> {
        if self.rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }

        let Some(range_lock_list) = self
            .path
            .inode()
            .fs_lock_context()
            .map(|c| c.range_lock_list())
        else {
            // The lock list is not present. So nothing is locked.
            lock.set_type(RangeLockType::Unlock);
            return Ok(lock);
        };

        let req_lock = range_lock_list.test_lock(lock);
        Ok(req_lock)
    }

    pub fn set_range_lock(&self, lock: &RangeLockItem, is_nonblocking: bool) -> Result<()> {
        match lock.type_() {
            RangeLockType::ReadLock => {
                if !self.rights.contains(Rights::READ) {
                    return_errno_with_message!(Errno::EBADF, "the file is not opened readable");
                }
            }
            RangeLockType::WriteLock => {
                if !self.rights.contains(Rights::WRITE) {
                    return_errno_with_message!(Errno::EBADF, "the file is not opened writable");
                }
            }
            RangeLockType::Unlock => {
                if self.rights.is_empty() {
                    return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
                }
            }
        }

        if RangeLockType::Unlock == lock.type_() {
            self.unlock_range_lock(lock);
            return Ok(());
        }

        let range_lock_list = self
            .path
            .inode()
            .fs_lock_context_or_init()
            .range_lock_list();
        range_lock_list.set_lock(lock, is_nonblocking)
    }

    fn release_range_locks(&self) {
        let range_lock = RangeLockItem::new(
            RangeLockType::Unlock,
            FileRange::new(0, OFFSET_MAX).unwrap(),
        );
        self.unlock_range_lock(&range_lock);
    }

    fn unlock_range_lock(&self, lock: &RangeLockItem) {
        if let Some(range_lock_list) = self
            .path
            .inode()
            .fs_lock_context()
            .map(|c| c.range_lock_list())
        {
            range_lock_list.unlock(lock);
        }
    }

    pub fn set_flock(&self, lock: FlockItem, is_nonblocking: bool) -> Result<()> {
        if self.rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }

        let flock_list = self.path.inode().fs_lock_context_or_init().flock_list();
        flock_list.set_lock(lock, is_nonblocking)
    }

    pub fn unlock_flock(&self) -> Result<()> {
        if self.rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }

        if let Some(flock_list) = self.path.inode().fs_lock_context().map(|c| c.flock_list()) {
            flock_list.unlock(self);
        }

        Ok(())
    }
}

impl Pollable for InodeHandle {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        if let Some(ref file_io) = self.file_io {
            return file_io.poll(mask, poller);
        }

        if self.rights.is_empty() {
            IoEvents::NVAL
        } else {
            let events = IoEvents::IN | IoEvents::OUT;
            events & mask
        }
    }
}

impl FileLike for InodeHandle {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        if !self.rights.contains(Rights::READ) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened readable");
        }

        let (inode_io, is_offset_aware) = self.inode_io_and_is_offset_aware();
        let status_flags = self.status_flags();

        if !is_offset_aware {
            return inode_io.read_at(0, writer, status_flags);
        }

        let mut offset = self.offset.lock();

        let len = inode_io.read_at(*offset, writer, status_flags)?;
        *offset += len;

        Ok(len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        if !self.rights.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened writable");
        }

        let (inode_io, is_offset_aware) = self.inode_io_and_is_offset_aware();
        let status_flags = self.status_flags();

        if !is_offset_aware {
            return inode_io.write_at(0, reader, status_flags);
        }

        let mut offset = self.offset.lock();

        // FIXME: How can we deal with the `O_APPEND` flag if `file_io` is set?
        if status_flags.contains(StatusFlags::O_APPEND) && self.file_io.is_none() {
            // FIXME: `O_APPEND` should ensure that new content is appended even if another process
            // is writing to the file concurrently.
            *offset = self.path.size();
        }

        let len = inode_io.write_at(*offset, reader, status_flags)?;
        *offset += len;

        Ok(len)
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        if !self.rights.contains(Rights::READ) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened readable");
        }

        let inode_io = self.inode_io_and_check_seekable()?;
        let status_flags = self.status_flags();

        inode_io.read_at(offset, writer, status_flags)
    }

    fn write_at(&self, mut offset: usize, reader: &mut VmReader) -> Result<usize> {
        if !self.rights.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened writable");
        }

        let inode_io = self.inode_io_and_check_seekable()?;
        let status_flags = self.status_flags();

        // FIXME: How can we deal with the `O_APPEND` flag if `file_io` is set?
        if status_flags.contains(StatusFlags::O_APPEND) && self.file_io.is_none() {
            // If the file has the `O_APPEND` flag, the offset is ignored.
            // FIXME: `O_APPEND` should ensure that new content is appended even if another process
            // is writing to the file concurrently.
            offset = self.path.size();
        }

        inode_io.write_at(offset, reader, status_flags)
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        if self.rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }

        if let Some(ref file_io) = self.file_io {
            return file_io.ioctl(raw_ioctl);
        }

        return_errno_with_message!(Errno::ENOTTY, "ioctl is not supported");
    }

    fn mappable(&self) -> Result<Mappable> {
        if self.rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }

        let inode = self.path.inode();
        if let Some(ref vmo) = inode.page_cache() {
            // If the inode has a page cache, it is a file-backed mapping and
            // we return the VMO as the mappable object.
            Ok(Mappable::Vmo(vmo.clone()))
        } else if let Some(ref file_io) = self.file_io {
            // Otherwise, it is a special file (e.g. device file) and we should
            // return the file-specific mappable object.
            file_io.mappable()
        } else {
            return_errno_with_message!(Errno::ENODEV, "the file is not mappable");
        }
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        if self.rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }
        if !self.rights.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EINVAL, "the file is not opened writable");
        }

        if self.status_flags().contains(StatusFlags::O_APPEND) {
            // FIXME: It's allowed to `ftruncate` an append-only file on Linux.
            return_errno_with_message!(Errno::EPERM, "can not resize append-only file");
        }
        self.path.inode().resize(new_size)
    }

    fn status_flags(&self) -> StatusFlags {
        self.status_flags.load(Ordering::Relaxed)
    }

    fn set_status_flags(&self, new_status_flags: StatusFlags) -> Result<()> {
        // TODO: Pipes currently require a special status flag check because
        // "packet" mode is not yet supported. Remove this check once "packet"
        // mode is implemented.
        if self
            .file_io
            .as_ref()
            .and_then(|file_io| (file_io.as_ref() as &dyn Any).downcast_ref::<PipeHandle>())
            .is_some()
        {
            crate::fs::pipe::check_status_flags(new_status_flags)?;
        }

        self.status_flags.store(new_status_flags, Ordering::Relaxed);

        Ok(())
    }

    fn access_mode(&self) -> AccessMode {
        self.rights.into()
    }

    fn seek(&self, pos: SeekFrom) -> Result<usize> {
        if self.rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }

        if let Some(ref file_io) = self.file_io {
            file_io.check_seekable()?;
            if file_io.is_offset_aware() {
                // TODO: Figure out whether we need to add support for seeking from the end of
                // special files.
                return do_seek_util(&self.offset, pos, None);
            } else {
                return Ok(0);
            }
        }

        let inode = self.path.inode();
        if !inode.type_().is_seekable() {
            return_errno_with_message!(Errno::ESPIPE, "seek is not supported");
        }
        do_seek_util(&self.offset, pos, inode.seek_end())
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        if !self.rights.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened writable");
        }

        let inode = self.path.inode().as_ref();
        let inode_type = inode.type_();

        // TODO: `fallocate` on pipe files also fails with `ESPIPE`.
        if inode_type == InodeType::NamedPipe {
            return_errno_with_message!(Errno::ESPIPE, "the inode is a FIFO file");
        }
        if !(inode_type == InodeType::File || inode_type == InodeType::Dir) {
            return_errno_with_message!(
                Errno::ENODEV,
                "the inode is not a regular file or a directory"
            );
        }

        let status_flags = self.status_flags();
        if status_flags.contains(StatusFlags::O_APPEND)
            && (mode == FallocMode::PunchHoleKeepSize
                || mode == FallocMode::CollapseRange
                || mode == FallocMode::InsertRange)
        {
            return_errno_with_message!(
                Errno::EPERM,
                "the flags do not work on the append-only file"
            );
        }
        if status_flags.contains(StatusFlags::O_DIRECT)
            || status_flags.contains(StatusFlags::O_PATH)
        {
            return_errno_with_message!(
                Errno::EBADF,
                "currently fallocate file with O_DIRECT or O_PATH is not supported"
            );
        }

        inode.fallocate(mode, offset, len)
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            inner: Arc<InodeHandle>,
            fd_flags: FdFlags,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut flags = self.inner.status_flags().bits() | self.inner.access_mode() as u32;
                if self.fd_flags.contains(FdFlags::CLOEXEC) {
                    flags |= CreationFlags::O_CLOEXEC.bits();
                }

                writeln!(f, "pos:\t{}", self.inner.offset())?;
                writeln!(f, "flags:\t0{:o}", flags)?;
                writeln!(f, "mnt_id:\t{}", self.inner.path.mount_node().id())?;
                writeln!(f, "ino:\t{}", self.inner.path.inode().ino())
            }
        }

        Box::new(FdInfo {
            inner: self,
            fd_flags,
        })
    }
}

impl Drop for InodeHandle {
    fn drop(&mut self) {
        self.release_range_locks();
        let _ = self.unlock_flock();
    }
}

impl Debug for InodeHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("InodeHandle")
            .field("path", &self.path)
            .field("offset", &self.offset())
            .field("status_flags", &self.status_flags())
            .field("rights", &self.rights)
            .finish_non_exhaustive()
    }
}

/// A trait for file-like objects that provide custom I/O operations.
///
/// This trait is typically implemented for special files like devices or
/// named pipes (FIFOs), which have behaviors different from regular on-disk files.
pub trait FileIo: Pollable + InodeIo + Any + Send + Sync + 'static {
    /// Checks whether the `seek()` operation should fail.
    fn check_seekable(&self) -> Result<()>;

    /// Returns whether the `read()`/`write()` operation should use and advance the offset.
    ///
    /// If [`FileIo::check_seekable`] succeeds but this method returns `false`,
    /// the offset in the `seek()` operation will be ignored.
    /// In that case, the `seek()` operation will do nothing but succeed.
    fn is_offset_aware(&self) -> bool;

    // See `FileLike::mappable`.
    fn mappable(&self) -> Result<Mappable> {
        return_errno_with_message!(Errno::EINVAL, "the file is not mappable");
    }

    fn ioctl(&self, _raw_ioctl: RawIoctl) -> Result<i32> {
        return_errno_with_message!(Errno::ENOTTY, "ioctl is not supported");
    }
}

fn do_seek_util(offset: &Mutex<usize>, pos: SeekFrom, end: Option<usize>) -> Result<usize> {
    let mut offset = offset.lock();

    let new_offset = match pos {
        SeekFrom::Start(off) => off,
        SeekFrom::End(diff) => {
            if let Some(end) = end {
                end.wrapping_add_signed(diff)
            } else {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "seeking the file from the end is not supported"
                );
            }
        }
        SeekFrom::Current(diff) => offset.wrapping_add_signed(diff),
    };

    // Invariant: `*offset <= isize::MAX as usize`.
    // TODO: Investigate whether `read`/`write` can break this invariant.
    if new_offset.cast_signed() < 0 {
        return_errno_with_message!(Errno::EINVAL, "the file offset cannot be negative");
    }

    *offset = new_offset;
    Ok(new_offset)
}
