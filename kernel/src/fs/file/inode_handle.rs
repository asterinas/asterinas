// SPDX-License-Identifier: MPL-2.0

//! Opened Inode-backed File Handle

use core::{fmt::Display, ops::Range, sync::atomic::Ordering};

use aster_rights::Rights;

use super::{
    AccessMode, AtomicStatusFlags, CreationFlags, FileLike, InodeType, Mappable, StatusFlags,
    file_table::FdFlags, flock::FlockItem,
};
use crate::{
    events::IoEvents,
    fs::{
        pipe::PipeHandle,
        utils::DirentVisitor,
        vfs::{
            inode::{FallocMode, FileOps},
            inode_ext::InodeExt,
            path::Path,
            range_lock::{FileRange, OFFSET_MAX, RangeLockItem, RangeLockType},
        },
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
};

pub struct InodeHandle {
    path: Path,
    /// `open_file` is similar to the `file_private` field in Linux's `file` structure. If
    /// `open_file` is `Some(_)`, typical file operations including `read`, `write`, `poll`,
    /// and `ioctl` will be provided by the per-open file object instead of `path`.
    open_file: Option<Box<dyn PerOpenFileOps>>,
    offset: Mutex<usize>,
    readahead: Mutex<ReadaheadState>,
    status_flags: AtomicStatusFlags,
    rights: Rights,
}

/// Maximum readahead window in pages.
const MAX_READAHEAD_PAGES: usize = 32;

/// Per-open-file readahead heuristic state.
#[derive(Debug, Default)]
struct ReadaheadState {
    /// Byte offset immediately after the most recent read.
    prev_pos: Option<usize>,
    /// Page index where the most recent readahead window started.
    start_page_idx: usize,
    /// Total size of the most recent readahead window in pages.
    nr_window_pages: usize,
    /// Page index where the next readahead window should be submitted.
    trigger_page_idx: usize,
}

impl ReadaheadState {
    /// Records a completed read.
    ///
    /// Returns the page-index range to prefetch.
    ///
    /// Reads are treated as sequential when their first page is the same as or
    /// immediately after the page containing the end of the previous read.
    fn on_read(&mut self, offset: usize, read_len: usize) -> Option<Range<usize>> {
        let read_end = offset + read_len;
        let read_start_idx = offset / PAGE_SIZE;
        let read_end_idx = read_end.div_ceil(PAGE_SIZE);

        if !self.is_sequential_read(read_start_idx) {
            self.prev_pos = Some(read_end);
            self.clear_window();
            return None;
        }

        self.prev_pos = Some(read_end);

        if self.nr_window_pages == 0 {
            return self.start_initial_window(read_start_idx, read_end_idx);
        }

        self.try_submit_next_window(read_end_idx)
    }

    /// Returns whether the read continues the previous page-level stream.
    fn is_sequential_read(&self, read_start_idx: usize) -> bool {
        let Some(prev_pos) = self.prev_pos else {
            return read_start_idx == 0;
        };

        let prev_page_idx = prev_pos / PAGE_SIZE;
        read_start_idx == prev_page_idx || read_start_idx == prev_page_idx + 1
    }

    /// Starts the initial window for a sequential stream.
    fn start_initial_window(
        &mut self,
        read_start_idx: usize,
        read_end_idx: usize,
    ) -> Option<Range<usize>> {
        let nr_window_pages = Self::initial_window_pages(read_end_idx - read_start_idx);
        let readahead_start_idx = read_end_idx;
        let window_end_idx = read_start_idx + nr_window_pages;

        self.start_page_idx = read_start_idx;
        self.nr_window_pages = nr_window_pages;
        self.trigger_page_idx = readahead_start_idx;

        if readahead_start_idx >= window_end_idx {
            return None;
        }

        Some(readahead_start_idx..window_end_idx)
    }

    /// Advances the window after the read reaches the trigger page.
    fn try_submit_next_window(&mut self, read_end_idx: usize) -> Option<Range<usize>> {
        let window_end_idx = self.start_page_idx + self.nr_window_pages;
        if read_end_idx <= self.trigger_page_idx {
            return None;
        }

        // If the read passed the whole window, skip pages that demand reads
        // should already have populated.
        let new_start_idx = window_end_idx.max(read_end_idx);
        let nr_new_window_pages = Self::next_window_pages(self.nr_window_pages);
        let new_end_idx = new_start_idx + nr_new_window_pages;

        self.start_page_idx = new_start_idx;
        self.nr_window_pages = nr_new_window_pages;
        self.trigger_page_idx = new_start_idx;

        Some(new_start_idx..new_end_idx)
    }

    /// Returns the initial total window size in pages.
    ///
    /// Reference:
    /// <https://elixir.bootlin.com/linux/v7.1-rc7/source/mm/readahead.c#L371>.
    fn initial_window_pages(nr_read_pages: usize) -> usize {
        debug_assert!(nr_read_pages > 0);

        if nr_read_pages > MAX_READAHEAD_PAGES / 4 {
            return MAX_READAHEAD_PAGES;
        }

        let base = nr_read_pages.next_power_of_two();
        if base <= MAX_READAHEAD_PAGES / 32 {
            base * 4
        } else if base <= MAX_READAHEAD_PAGES / 4 {
            base * 2
        } else {
            MAX_READAHEAD_PAGES
        }
    }

    /// Returns the next total window size in pages.
    ///
    /// Reference:
    /// <https://elixir.bootlin.com/linux/v7.1-rc7/source/mm/readahead.c#L390>.
    fn next_window_pages(nr_current_pages: usize) -> usize {
        if nr_current_pages < MAX_READAHEAD_PAGES / 16 {
            nr_current_pages * 4
        } else if nr_current_pages <= MAX_READAHEAD_PAGES / 2 {
            nr_current_pages * 2
        } else {
            MAX_READAHEAD_PAGES
        }
    }

    /// Clears the active readahead window.
    fn clear_window(&mut self) {
        self.start_page_idx = 0;
        self.nr_window_pages = 0;
        self.trigger_page_idx = 0;
    }
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
        let (open_file, rights) = if status_flags.contains(StatusFlags::O_PATH) {
            (None, Rights::empty())
        } else if inode.type_() == InodeType::Dir && access_mode.is_writable() {
            return_errno_with_message!(Errno::EISDIR, "a directory cannot be opened writable");
        } else {
            let open_file = inode.open(access_mode, status_flags).transpose()?;
            let rights = Rights::from(access_mode);
            (open_file, rights)
        };

        Ok(Self {
            path,
            open_file,
            offset: Mutex::new(0),
            readahead: Mutex::new(ReadaheadState::default()),
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

    pub(in crate::fs) fn rights(&self) -> Rights {
        self.rights
    }

    fn file_ops_and_is_offset_aware(&self) -> (&dyn FileOps, bool) {
        if let Some(ref open_file) = self.open_file {
            let is_offset_aware = open_file.is_offset_aware();
            return (open_file.as_ref(), is_offset_aware);
        }

        let inode = self.path.inode();
        let is_offset_aware = inode.type_().is_seekable();
        (inode.as_ref(), is_offset_aware)
    }

    /// Returns the `FileOps` for positional I/O, rejecting files
    /// that do not support `pread`/`pwrite`.
    fn file_ops_for_positional_io(&self) -> Result<&dyn FileOps> {
        if let Some(ref open_file) = self.open_file {
            open_file.check_positional_io()?;
            return Ok(open_file.as_ref());
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

        let file_ops: &dyn FileOps = if let Some(ref open_file) = self.open_file {
            open_file.as_ref()
        } else {
            self.path.inode().as_ref()
        };
        let mut offset = self.offset.lock();
        let read_cnt = file_ops.readdir_at(*offset, visitor)?;
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

    pub fn release_range_locks(&self) {
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

    pub fn downcast_open_file<T: 'static>(&self) -> Result<Option<&T>> {
        if self.rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }

        let Some(open_file) = self.open_file.as_ref() else {
            return Ok(None);
        };

        Ok((open_file.as_ref() as &dyn Any).downcast_ref::<T>())
    }

    fn maybe_readahead(&self, offset: usize, read_len: usize, status_flags: StatusFlags) {
        if read_len == 0 || self.open_file.is_some() || status_flags.contains(StatusFlags::O_DIRECT)
        {
            return;
        }
        let Some(page_cache) = self.path.inode().page_cache() else {
            return;
        };

        // If another reader is updating the heuristic state, skip this opportunity and let
        // the demand read complete without waiting.
        let Some(mut readahead) = self.readahead.try_lock() else {
            return;
        };
        let Some(page_idx_range) = readahead.on_read(offset, read_len) else {
            return;
        };
        drop(readahead);

        let file_size = self.path.size();
        let file_end_idx = file_size.div_ceil(PAGE_SIZE);
        let readahead_end_idx = page_idx_range.end.min(file_end_idx);
        if page_idx_range.start >= readahead_end_idx {
            return;
        }

        // Readahead must not affect the result of the completed demand read.
        // Cache population failures only lose this speculative optimization.
        let _ = page_cache.readahead(page_idx_range.start..readahead_end_idx);
    }
}

impl Pollable for InodeHandle {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        if let Some(ref open_file) = self.open_file {
            return open_file.poll(mask, poller);
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

        let (file_ops, is_offset_aware) = self.file_ops_and_is_offset_aware();
        let status_flags = self.status_flags();

        if !is_offset_aware {
            return file_ops.read_at(0, writer, status_flags);
        }

        let (old_offset, len) = {
            let mut offset = self.offset.lock();
            let old_offset = *offset;
            let len = file_ops.read_at(old_offset, writer, status_flags)?;
            *offset += len;
            (old_offset, len)
        };
        self.maybe_readahead(old_offset, len, status_flags);

        Ok(len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        if !self.rights.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened writable");
        }

        let (file_ops, is_offset_aware) = self.file_ops_and_is_offset_aware();
        let status_flags = self.status_flags();

        if !is_offset_aware {
            return file_ops.write_at(0, reader, status_flags);
        }

        let mut offset = self.offset.lock();

        // FIXME: How can we deal with the `O_APPEND` flag if `open_file` is set?
        if status_flags.contains(StatusFlags::O_APPEND) && self.open_file.is_none() {
            // FIXME: `O_APPEND` should ensure that new content is appended even if another process
            // is writing to the file concurrently.
            *offset = self.path.size();
        }

        let len = file_ops.write_at(*offset, reader, status_flags)?;
        *offset += len;

        Ok(len)
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let file_ops = self.file_ops_for_positional_io()?;
        if !self.rights.contains(Rights::READ) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened readable");
        }

        let status_flags = self.status_flags();

        let len = file_ops.read_at(offset, writer, status_flags)?;
        self.maybe_readahead(offset, len, status_flags);

        Ok(len)
    }

    fn write_at(&self, mut offset: usize, reader: &mut VmReader) -> Result<usize> {
        let file_ops = self.file_ops_for_positional_io()?;
        if !self.rights.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened writable");
        }

        let status_flags = self.status_flags();

        // FIXME: How can we deal with the `O_APPEND` flag if `open_file` is set?
        if status_flags.contains(StatusFlags::O_APPEND) && self.open_file.is_none() {
            // If the file has the `O_APPEND` flag, the offset is ignored.
            // FIXME: `O_APPEND` should ensure that new content is appended even if another process
            // is writing to the file concurrently.
            offset = self.path.size();
        }

        file_ops.write_at(offset, reader, status_flags)
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        if self.rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }

        if let Some(ref open_file) = self.open_file {
            return open_file.ioctl(raw_ioctl);
        }

        return_errno_with_message!(Errno::ENOTTY, "ioctl is not supported");
    }

    fn mappable(&self) -> Result<Mappable> {
        if self.rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }

        let inode = self.path.inode();
        if let Some(ref page_cache) = inode.page_cache() {
            // If the inode has a page cache, it is a file-backed mapping and
            // we return the VMO as the mappable object.
            Ok(Mappable::Vmo(page_cache.as_vmo().clone()))
        } else if let Some(ref open_file) = self.open_file {
            // Otherwise, it is a special file (e.g. device file) and we should
            // return the file-specific mappable object.
            open_file.mappable()
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
            .open_file
            .as_ref()
            .and_then(|open_file| (open_file.as_ref() as &dyn Any).downcast_ref::<PipeHandle>())
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

        if let Some(ref open_file) = self.open_file {
            open_file.check_seekable()?;
            if open_file.is_offset_aware() {
                // TODO: Figure out whether we need to add support for seeking from the end of
                // special files.
                return do_seek_util(&self.offset, pos, open_file.seek_end()?);
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

/// Describes the position to seek from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeekFrom {
    Start(usize),
    End(isize),
    Current(isize),
}

/// File operations for one opened file description.
///
/// A per-open file object can hold file-description-specific state and override
/// operations that are not purely inode-backed, such as state and operations for
/// devices, pipes, namespace files, and procfs files.
pub trait PerOpenFileOps: Pollable + FileOps + Any + Send + Sync + 'static {
    /// Checks whether the `seek()` operation should fail.
    fn check_seekable(&self) -> Result<()>;

    /// Returns whether the `read()`/`write()` operation should use and advance the offset.
    ///
    /// If [`PerOpenFileOps::check_seekable`] succeeds but this method returns `false`,
    /// the offset in the `seek()` operation will be ignored.
    /// In that case, the `seek()` operation will do nothing but succeed.
    fn is_offset_aware(&self) -> bool;

    /// Checks whether positional I/O (`pread`/`pwrite`) is supported.
    ///
    /// The default delegates to [`check_seekable`], which is correct for
    /// most files. Override this for files that support positional I/O
    /// but not seeking (e.g., nsfs).
    ///
    /// [`check_seekable`]: PerOpenFileOps::check_seekable
    fn check_positional_io(&self) -> Result<()> {
        self.check_seekable()
    }

    /// Returns the end position for [`SeekFrom::End`].
    ///
    /// This is intentionally separate from `Inode::seek_end`. Both `Inode`
    /// and [`PerOpenFileOps`] need `SEEK_END` support, but `Inode::seek_end`
    /// has an inode-specific default implementation, so the two cannot be
    /// cleanly unified under [`FileOps`].
    fn seek_end(&self) -> Result<Option<usize>> {
        Ok(None)
    }

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
