// SPDX-License-Identifier: MPL-2.0

#![expect(unused_variables)]

//! Opened Inode-backed File Handle

mod dyn_cap;
mod static_cap;

use core::sync::atomic::{AtomicU32, Ordering};

use aster_rights::Rights;
use inherit_methods_macro::inherit_methods;

use crate::{
    events::IoEvents,
    fs::{
        file_handle::{FileLike, MemoryToMap},
        path::Path,
        utils::{
            AccessMode, DirentVisitor, FallocMode, FileRange, FlockItem, FlockList, Inode,
            InodeMode, InodeType, IoctlCmd, Metadata, RangeLockItem, RangeLockItemBuilder,
            RangeLockList, RangeLockType, SeekFrom, StatusFlags, OFFSET_MAX,
        },
    },
    prelude::*,
    process::{
        signal::{PollHandle, Pollable},
        Gid, Uid,
    },
};

#[derive(Debug)]
pub struct InodeHandle<R = Rights>(Arc<InodeHandle_>, R);

struct InodeHandle_ {
    path: Path,
    /// `file_io` is Similar to `file_private` field in `file` structure in linux. If
    /// `file_io` is Some, typical file operations including `read`, `write`, `poll`,
    /// `ioctl` will be provided by `file_io`, instead of `path`.
    file_io: Option<Arc<dyn FileIo>>,
    offset: Mutex<usize>,
    access_mode: AccessMode,
    status_flags: AtomicU32,
}

impl InodeHandle_ {
    pub fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        if let Some(ref file_io) = self.file_io {
            return file_io.read(writer);
        }

        if !self.path.inode().is_seekable() {
            return self.read_at(0, writer);
        }

        let mut offset = self.offset.lock();

        let len = self.read_at(*offset, writer)?;

        *offset += len;
        Ok(len)
    }

    pub fn write(&self, reader: &mut VmReader) -> Result<usize> {
        if let Some(ref file_io) = self.file_io {
            return file_io.write(reader);
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

    pub fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        if let Some(ref file_io) = self.file_io {
            todo!("support read_at for FileIo");
        }

        if self.status_flags().contains(StatusFlags::O_DIRECT) {
            self.path.inode().read_direct_at(offset, writer)
        } else {
            self.path.inode().read_at(offset, writer)
        }
    }

    pub fn write_at(&self, mut offset: usize, reader: &mut VmReader) -> Result<usize> {
        if let Some(ref file_io) = self.file_io {
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

    pub fn seek(&self, pos: SeekFrom) -> Result<usize> {
        do_seek_util(self.path.inode(), &self.offset, pos)
    }

    pub fn offset(&self) -> usize {
        let offset = self.offset.lock();
        *offset
    }

    pub fn resize(&self, new_size: usize) -> Result<()> {
        do_resize_util(self.path.inode(), self.status_flags(), new_size)
    }

    pub fn access_mode(&self) -> AccessMode {
        self.access_mode
    }

    pub fn status_flags(&self) -> StatusFlags {
        let bits = self.status_flags.load(Ordering::Relaxed);
        StatusFlags::from_bits(bits).unwrap()
    }

    pub fn set_status_flags(&self, new_status_flags: StatusFlags) {
        self.status_flags
            .store(new_status_flags.bits(), Ordering::Relaxed);
    }

    pub fn readdir(&self, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let mut offset = self.offset.lock();
        let read_cnt = self.path.inode().readdir_at(*offset, visitor)?;
        *offset += read_cnt;
        Ok(read_cnt)
    }

    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        if let Some(ref file_io) = self.file_io {
            return file_io.poll(mask, poller);
        }

        self.path.inode().poll(mask, poller)
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        do_fallocate_util(self.path.inode(), self.status_flags(), mode, offset, len)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        if let Some(ref file_io) = self.file_io {
            return file_io.ioctl(cmd, arg);
        }

        self.path.inode().ioctl(cmd, arg)
    }

    fn mmap(&self) -> Result<MemoryToMap> {
        let inode = self.path.inode();
        if inode.page_cache().is_some() {
            // If the inode has a page cache, it is a file-backed mapping and
            // we directly return the corresponding inode.
            Ok(MemoryToMap::PageCache(inode.clone()))
        } else if let Some(ref file_io) = self.file_io {
            // Otherwise, let the file-specific mmap handle.
            file_io.mmap()
        } else {
            return_errno_with_message!(Errno::EINVAL, "mmap is not supported");
        }
    }

    fn test_range_lock(&self, lock: RangeLockItem) -> Result<RangeLockItem> {
        let mut req_lock = lock.clone();
        if let Some(extension) = self.path.inode().extension() {
            if let Some(range_lock_list) = extension.get::<RangeLockList>() {
                req_lock = range_lock_list.test_lock(lock);
            } else {
                // The range lock could be placed if there is no lock list
                req_lock.set_type(RangeLockType::Unlock);
            }
        } else {
            debug!("Inode extension is not supported, the lock could be placed");
            // Some file systems may not support range lock like procfs and sysfs
            // Returns Ok if extension is not supported.
            req_lock.set_type(RangeLockType::Unlock);
        }
        Ok(req_lock)
    }

    fn set_range_lock(&self, lock: &RangeLockItem, is_nonblocking: bool) -> Result<()> {
        if RangeLockType::Unlock == lock.type_() {
            self.unlock_range_lock(lock);
            return Ok(());
        }

        self.check_range_lock_with_access_mode(lock)?;
        if let Some(extension) = self.path.inode().extension() {
            let range_lock_list = match extension.get::<RangeLockList>() {
                Some(list) => list,
                None => extension.get_or_put_default::<RangeLockList>(),
            };

            range_lock_list.set_lock(lock, is_nonblocking)
        } else {
            debug!("Inode extension is not supported, let the lock could be acquired");
            // Some file systems may not support range lock like procfs and sysfs
            // Returns Ok if extension is not supported.
            Ok(())
        }
    }

    fn release_range_locks(&self) {
        if self.path.inode().extension().is_none() {
            return;
        }

        let range_lock = RangeLockItemBuilder::new()
            .type_(RangeLockType::Unlock)
            .range(FileRange::new(0, OFFSET_MAX).unwrap())
            .build()
            .unwrap();
        self.unlock_range_lock(&range_lock);
    }

    fn unlock_range_lock(&self, lock: &RangeLockItem) {
        if let Some(extension) = self.path.inode().extension() {
            if let Some(range_lock_list) = extension.get::<RangeLockList>() {
                range_lock_list.unlock(lock);
            }
        }
    }

    fn check_range_lock_with_access_mode(&self, lock: &RangeLockItem) -> Result<()> {
        match lock.type_() {
            RangeLockType::ReadLock => {
                if !self.access_mode().is_readable() {
                    return_errno_with_message!(Errno::EBADF, "file not readable");
                }
            }
            RangeLockType::WriteLock => {
                if !self.access_mode().is_writable() {
                    return_errno_with_message!(Errno::EBADF, "file not writable");
                }
            }
            _ => (),
        }
        Ok(())
    }

    fn set_flock(&self, lock: FlockItem, is_nonblocking: bool) -> Result<()> {
        if let Some(extension) = self.path.inode().extension() {
            let flock_list = match extension.get::<FlockList>() {
                Some(list) => list,
                None => extension.get_or_put_default::<FlockList>(),
            };

            flock_list.set_lock(lock, is_nonblocking)
        } else {
            debug!("Inode extension is not supported, let the lock could be acquired");
            // Some file systems may not support flock like procfs and sysfs
            // Returns Ok if extension is not supported.
            Ok(())
        }
    }

    fn unlock_flock<R>(&self, req_owner: &InodeHandle<R>) {
        if let Some(extension) = self.path.inode().extension() {
            if let Some(flock_list) = extension.get::<FlockList>() {
                flock_list.unlock(req_owner);
            }
        }
    }
}

#[inherit_methods(from = "self.path")]
impl InodeHandle_ {
    pub fn size(&self) -> usize;
    pub fn metadata(&self) -> Metadata;
    pub fn mode(&self) -> Result<InodeMode>;
    pub fn set_mode(&self, mode: InodeMode) -> Result<()>;
    pub fn owner(&self) -> Result<Uid>;
    pub fn set_owner(&self, uid: Uid) -> Result<()>;
    pub fn group(&self) -> Result<Gid>;
    pub fn set_group(&self, gid: Gid) -> Result<()>;
}

impl Debug for InodeHandle_ {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("InodeHandle_")
            .field("path", &self.path)
            .field("offset", &self.offset())
            .field("access_mode", &self.access_mode())
            .field("status_flags", &self.status_flags())
            .finish()
    }
}

/// Methods for both dyn and static
impl<R> InodeHandle<R> {
    pub fn path(&self) -> &Path {
        &self.0.path
    }

    pub fn test_range_lock(&self, lock: RangeLockItem) -> Result<RangeLockItem> {
        self.0.test_range_lock(lock)
    }

    pub fn set_range_lock(&self, lock: &RangeLockItem, is_nonblocking: bool) -> Result<()> {
        self.0.set_range_lock(lock, is_nonblocking)
    }

    pub fn release_range_locks(&self) {
        self.0.release_range_locks()
    }

    pub fn set_flock(&self, lock: FlockItem, is_nonblocking: bool) -> Result<()> {
        self.0.set_flock(lock, is_nonblocking)
    }

    pub fn unlock_flock(&self) {
        self.0.unlock_flock(self);
    }

    pub fn offset(&self) -> usize {
        self.0.offset()
    }
}

impl<R> Drop for InodeHandle<R> {
    fn drop(&mut self) {
        self.release_range_locks();
        self.unlock_flock();
    }
}

pub trait FileIo: Pollable + Send + Sync + 'static {
    fn read(&self, writer: &mut VmWriter) -> Result<usize>;

    fn write(&self, reader: &mut VmReader) -> Result<usize>;

    /// See [`FileLike::mmap`].
    fn mmap(&self) -> Result<MemoryToMap> {
        return_errno_with_message!(Errno::EINVAL, "mmap is not supported");
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        return_errno_with_message!(Errno::EINVAL, "ioctl is not supported");
    }
}

pub fn do_seek_util(inode: &Arc<dyn Inode>, offset: &Mutex<usize>, pos: SeekFrom) -> Result<usize> {
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

pub fn do_fallocate_util(
    inode: &Arc<dyn Inode>,
    status_flags: StatusFlags,
    mode: FallocMode,
    offset: usize,
    len: usize,
) -> Result<()> {
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

pub fn do_resize_util(
    inode: &Arc<dyn Inode>,
    status_flags: StatusFlags,
    new_size: usize,
) -> Result<()> {
    if status_flags.contains(StatusFlags::O_APPEND) {
        // FIXME: It's allowed to `ftruncate` an append-only file on Linux.
        return_errno_with_message!(Errno::EPERM, "can not resize append-only file");
    }
    inode.resize(new_size)
}
