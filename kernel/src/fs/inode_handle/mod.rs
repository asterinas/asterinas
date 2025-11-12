// SPDX-License-Identifier: MPL-2.0

//! Opened Inode-backed File Handle

mod dyn_cap;

use core::sync::atomic::{AtomicU32, Ordering};

pub use dyn_cap::InodeHandle;
use inherit_methods_macro::inherit_methods;

use crate::{
    events::IoEvents,
    fs::{
        file_handle::Mappable,
        path::Path,
        utils::{
            DirentVisitor, FallocMode, FileRange, FlockItem, FlockList, Inode, InodeMode,
            InodeType, IoctlCmd, Metadata, RangeLockItem, RangeLockList, RangeLockType, SeekFrom,
            StatusFlags, OFFSET_MAX,
        },
    },
    prelude::*,
    process::{
        signal::{PollHandle, Pollable},
        Gid, Uid,
    },
};

struct HandleInner {
    path: Path,
    /// `file_io` is similar to the `file_private` field in Linux's `file` structure. If `file_io`
    /// is `Some(_)`, typical file operations including `read`, `write`, `poll`, and `ioctl` will
    /// be provided by `file_io`, instead of `path`.
    file_io: Option<Arc<dyn FileIo>>,
    offset: Mutex<usize>,
    status_flags: AtomicU32,
}

impl HandleInner {
    pub(self) fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        if let Some(ref file_io) = self.file_io {
            return file_io.read(writer, self.status_flags());
        }

        if !self.path.inode().is_seekable() {
            return self.read_at(0, writer);
        }

        let mut offset = self.offset.lock();

        let len = self.read_at(*offset, writer)?;

        *offset += len;
        Ok(len)
    }

    pub(self) fn write(&self, reader: &mut VmReader) -> Result<usize> {
        if let Some(ref file_io) = self.file_io {
            return file_io.write(reader, self.status_flags());
        }

        if !self.path.inode().is_seekable() {
            return self.write_at(0, reader);
        }

        let mut offset = self.offset.lock();

        if self.status_flags().contains(StatusFlags::O_APPEND) {
            *offset = self.path.size();
        }

        let len = self.write_at(*offset, reader)?;

        *offset += len;
        Ok(len)
    }

    pub(self) fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        if let Some(ref _file_io) = self.file_io {
            todo!("support read_at for FileIo");
        }

        if self.status_flags().contains(StatusFlags::O_DIRECT) {
            self.path.inode().read_direct_at(offset, writer)
        } else {
            self.path.inode().read_at(offset, writer)
        }
    }

    pub(self) fn write_at(&self, mut offset: usize, reader: &mut VmReader) -> Result<usize> {
        if let Some(ref _file_io) = self.file_io {
            todo!("support write_at for FileIo");
        }

        let status_flags = self.status_flags();
        if status_flags.contains(StatusFlags::O_APPEND) {
            // If the file has the O_APPEND flag, the offset is ignored
            offset = self.path.size();
        }

        if status_flags.contains(StatusFlags::O_DIRECT) {
            self.path.inode().write_direct_at(offset, reader)
        } else {
            self.path.inode().write_at(offset, reader)
        }
    }

    pub(self) fn seek(&self, pos: SeekFrom) -> Result<usize> {
        do_seek_util(self.path.inode().as_ref(), &self.offset, pos)
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

        self.path.inode().poll(mask, poller)
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

    pub(self) fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        if let Some(ref file_io) = self.file_io {
            return file_io.ioctl(cmd, arg);
        }

        self.path.inode().ioctl(cmd, arg)
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

#[inherit_methods(from = "self.path")]
impl HandleInner {
    pub(self) fn size(&self) -> usize;
    pub(self) fn metadata(&self) -> Metadata;
    pub(self) fn mode(&self) -> Result<InodeMode>;
    pub(self) fn set_mode(&self, mode: InodeMode) -> Result<()>;
    pub(self) fn owner(&self) -> Result<Uid>;
    pub(self) fn set_owner(&self, uid: Uid) -> Result<()>;
    pub(self) fn group(&self) -> Result<Gid>;
    pub(self) fn set_group(&self, gid: Gid) -> Result<()>;
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
//
// TODO: The `status_flags` parameter in `read` and `write` may need to be stored directly
// in the `FileIo`. We need further refactoring to find an appropriate way to enable `FileIo`
// to utilize the information in the `HandleInner`.
pub trait FileIo: Pollable + Send + Sync + 'static {
    /// Reads data from the file into the given `VmWriter`.
    fn read(&self, writer: &mut VmWriter, status_flags: StatusFlags) -> Result<usize>;

    /// Writes data from the given `VmReader` into the file.
    fn write(&self, reader: &mut VmReader, status_flags: StatusFlags) -> Result<usize>;

    // See `FileLike::mappable`.
    fn mappable(&self) -> Result<Mappable> {
        return_errno_with_message!(Errno::EINVAL, "the file is not mappable");
    }

    fn ioctl(&self, _cmd: IoctlCmd, _arg: usize) -> Result<i32> {
        return_errno_with_message!(Errno::ENOTTY, "ioctl is not supported");
    }
}

pub(super) fn do_seek_util(
    inode: &dyn Inode,
    offset: &Mutex<usize>,
    pos: SeekFrom,
) -> Result<usize> {
    let mut offset = offset.lock();
    let new_offset: isize = match pos {
        SeekFrom::Start(off /* as usize */) => {
            if off > isize::MAX as usize {
                return_errno_with_message!(Errno::EINVAL, "file offset is too large");
            }
            off as isize
        }
        SeekFrom::End(off /* as isize */) => {
            let file_size = inode.size() as isize;
            assert!(file_size >= 0);
            file_size
                .checked_add(off)
                .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "file offset overflow"))?
        }
        SeekFrom::Current(off /* as isize */) => (*offset as isize)
            .checked_add(off)
            .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "file offset overflow"))?,
    };
    if new_offset < 0 {
        return_errno_with_message!(Errno::EINVAL, "file offset must not be negative");
    }
    // Invariant: 0 <= new_offset <= isize::MAX
    let new_offset = new_offset as usize;
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
