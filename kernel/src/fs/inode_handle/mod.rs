// SPDX-License-Identifier: MPL-2.0

//! Opened Inode-backed File Handle

mod dyn_cap;

use core::sync::atomic::{AtomicU32, Ordering};

pub use dyn_cap::InodeHandle;

use super::utils::InodeIo;
use crate::{
    events::IoEvents,
    fs::{
        file_handle::Mappable,
        path::Path,
        utils::{
            DirentVisitor, FallocMode, FileRange, FlockItem, FlockList, Inode, InodeType,
            OFFSET_MAX, RangeLockItem, RangeLockList, RangeLockType, SeekFrom, StatusFlags,
        },
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
};

struct HandleInner {
    path: Path,
    /// `file_io` is similar to the `file_private` field in Linux's `file` structure. If `file_io`
    /// is `Some(_)`, typical file operations including `read`, `write`, `poll`, and `ioctl` will
    /// be provided by `file_io`, instead of `path`.
    file_io: Option<Box<dyn FileIo>>,
    offset: Mutex<usize>,
    status_flags: AtomicU32,
}

impl HandleInner {
    pub(self) fn read(&self, writer: &mut VmWriter) -> Result<usize> {
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

    pub(self) fn write(&self, reader: &mut VmReader) -> Result<usize> {
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

    fn inode_io_and_is_offset_aware(&self) -> (&dyn InodeIo, bool) {
        if let Some(ref file_io) = self.file_io {
            let is_offset_aware = file_io.is_offset_aware();
            return (file_io.as_ref(), is_offset_aware);
        }

        let inode = self.path.inode();
        let is_offset_aware = inode.type_().is_seekable();
        (inode.as_ref(), is_offset_aware)
    }

    pub(self) fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let inode_io = self.inode_io_and_check_seekable()?;
        let status_flags = self.status_flags();

        inode_io.read_at(offset, writer, status_flags)
    }

    pub(self) fn write_at(&self, mut offset: usize, reader: &mut VmReader) -> Result<usize> {
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

    pub(self) fn seek(&self, pos: SeekFrom) -> Result<usize> {
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

    pub(self) fn offset(&self) -> usize {
        let offset = self.offset.lock();
        *offset
    }

    pub(self) fn resize(&self, new_size: usize) -> Result<()> {
        do_resize_util(self.path.inode().as_ref(), self.status_flags(), new_size)
    }

    pub(self) fn status_flags(&self) -> StatusFlags {
        let bits = self.status_flags.load(Ordering::Relaxed);
        StatusFlags::from_bits(bits).unwrap()
    }

    pub(self) fn set_status_flags(&self, new_status_flags: StatusFlags) {
        self.status_flags
            .store(new_status_flags.bits(), Ordering::Relaxed);
    }

    pub(self) fn readdir(&self, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let mut offset = self.offset.lock();
        let read_cnt = self.path.inode().readdir_at(*offset, visitor)?;
        *offset += read_cnt;
        Ok(read_cnt)
    }

    pub(self) fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        if let Some(ref file_io) = self.file_io {
            return file_io.poll(mask, poller);
        }

        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }

    pub(self) fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        do_fallocate_util(
            self.path.inode().as_ref(),
            self.status_flags(),
            mode,
            offset,
            len,
        )
    }

    pub(self) fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        if let Some(ref file_io) = self.file_io {
            return file_io.ioctl(raw_ioctl);
        }

        return_errno_with_message!(Errno::ENOTTY, "ioctl is not supported");
    }

    pub(self) fn mappable(&self) -> Result<Mappable> {
        let inode = self.path.inode();
        if inode.page_cache().is_some() {
            // If the inode has a page cache, it is a file-backed mapping and
            // we directly return the corresponding inode.
            Ok(Mappable::Inode(inode.clone()))
        } else if let Some(ref file_io) = self.file_io {
            // Otherwise, it is a special file (e.g. device file) and we should
            // return the file-specific mappable object.
            file_io.mappable()
        } else {
            return_errno_with_message!(Errno::ENODEV, "the file is not mappable");
        }
    }

    pub(self) fn test_range_lock(&self, mut lock: RangeLockItem) -> Result<RangeLockItem> {
        let Some(extension) = self.path.inode().extension() else {
            // Range locks are not supported. So nothing is locked.
            lock.set_type(RangeLockType::Unlock);
            return Ok(lock);
        };

        let Some(range_lock_list) = extension.get::<RangeLockList>() else {
            // The lock list is not present. So nothing is locked.
            lock.set_type(RangeLockType::Unlock);
            return Ok(lock);
        };

        let req_lock = range_lock_list.test_lock(lock);
        Ok(req_lock)
    }

    pub(self) fn set_range_lock(&self, lock: &RangeLockItem, is_nonblocking: bool) -> Result<()> {
        if RangeLockType::Unlock == lock.type_() {
            self.unlock_range_lock(lock);
            return Ok(());
        }

        let Some(extension) = self.path.inode().extension() else {
            // TODO: Figure out whether range locks are supported on all inodes.
            warn!("the inode does not have support for range locks; this operation will fail");
            return_errno_with_message!(Errno::ENOLCK, "range locks are not supported");
        };

        let range_lock_list = match extension.get::<RangeLockList>() {
            Some(list) => list,
            None => extension.get_or_put_default::<RangeLockList>(),
        };
        range_lock_list.set_lock(lock, is_nonblocking)
    }

    pub(self) fn release_range_locks(&self) {
        if self.path.inode().extension().is_none() {
            return;
        }

        let range_lock = RangeLockItem::new(
            RangeLockType::Unlock,
            FileRange::new(0, OFFSET_MAX).unwrap(),
        );
        self.unlock_range_lock(&range_lock);
    }

    pub(self) fn unlock_range_lock(&self, lock: &RangeLockItem) {
        if let Some(extension) = self.path.inode().extension()
            && let Some(range_lock_list) = extension.get::<RangeLockList>()
        {
            range_lock_list.unlock(lock);
        }
    }

    pub(self) fn set_flock(&self, lock: FlockItem, is_nonblocking: bool) -> Result<()> {
        let Some(extension) = self.path.inode().extension() else {
            // TODO: Figure out whether flocks are supported on all inodes.
            warn!("the inode does not have support for flocks; this operation will fail");
            return_errno_with_message!(Errno::ENOLCK, "flocks are not supported");
        };

        let flock_list = match extension.get::<FlockList>() {
            Some(list) => list,
            None => extension.get_or_put_default::<FlockList>(),
        };
        flock_list.set_lock(lock, is_nonblocking)
    }

    pub(self) fn unlock_flock(&self, req_owner: &InodeHandle) {
        if let Some(extension) = self.path.inode().extension()
            && let Some(flock_list) = extension.get::<FlockList>()
        {
            flock_list.unlock(req_owner);
        }
    }
}

impl Debug for HandleInner {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("HandleInner")
            .field("path", &self.path)
            .field("offset", &self.offset())
            .field("status_flags", &self.status_flags())
            .finish_non_exhaustive()
    }
}

/// A trait for file-like objects that provide custom I/O operations.
///
/// This trait is typically implemented for special files like devices or
/// named pipes (FIFOs), which have behaviors different from regular on-disk files.
pub trait FileIo: Pollable + InodeIo + Send + Sync + 'static {
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

pub(super) fn do_seek_util(
    offset: &Mutex<usize>,
    pos: SeekFrom,
    end: Option<usize>,
) -> Result<usize> {
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

pub(super) fn do_fallocate_util(
    inode: &dyn Inode,
    status_flags: StatusFlags,
    mode: FallocMode,
    offset: usize,
    len: usize,
) -> Result<()> {
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
    if status_flags.contains(StatusFlags::O_DIRECT) || status_flags.contains(StatusFlags::O_PATH) {
        return_errno_with_message!(
            Errno::EBADF,
            "currently fallocate file with O_DIRECT or O_PATH is not supported"
        );
    }

    inode.fallocate(mode, offset, len)
}

pub(super) fn do_resize_util(
    inode: &dyn Inode,
    status_flags: StatusFlags,
    new_size: usize,
) -> Result<()> {
    if status_flags.contains(StatusFlags::O_APPEND) {
        // FIXME: It's allowed to `ftruncate` an append-only file on Linux.
        return_errno_with_message!(Errno::EPERM, "can not resize append-only file");
    }
    inode.resize(new_size)
}
