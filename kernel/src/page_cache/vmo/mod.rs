// SPDX-License-Identifier: MPL-2.0

//! Lower-level virtual memory object machinery used by the page cache.
//!
//! This submodule defines [`Vmo`] and [`VmoOptions`], the generic memory-object
//! layer underneath [`crate::page_cache::PageCache`]. A [`Vmo`] owns a sparse
//! array of pages, commits them on demand, and optionally cooperates with a
//! [`crate::page_cache::PageCacheBackend`] to populate or write back disk-backed
//! pages.
//!
//! # When to Use This Module
//!
//! - Filesystem buffered I/O, invalidation, and resize should usually stay at
//!   [`crate::page_cache::PageCache`].
//! - Memory-mapping, page-fault, and other lower-level VM paths that need
//!   direct page-oriented access should work with [`Vmo`].
//!
//! # Internal Layering
//!
//! Within the page-cache subsystem:
//!
//! - [`crate::page_cache::PageCache`] adds filesystem-facing policy and APIs.
//! - [`Vmo`] is the reusable page-array and commit engine.
//! - [`crate::page_cache::cache_page`] defines the individual page states and
//!   backend hand-off helpers.
//!
//! See [`Vmo`] for the detailed memory-object contract.

use core::{
    ops::{Deref, Range},
    sync::atomic::{AtomicIsize, AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use aster_block::bio::{BioStatus, BioWaiter};
use ostd::{
    mm::{VmIo, VmIoFill, VmReader, VmWriter, io_util::HasVmReaderWriter},
    task::disable_preempt,
};
use xarray::{Cursor, LockedXArray, XArray};

use crate::{
    page_cache::{
        CachePage, CachePageExt, PageCacheBackend,
        cache_page::{LockedCachePage, clear_writing_back},
    },
    prelude::*,
};

mod options;

pub use options::VmoOptions;

/// Page-indexed memory object used by the page cache and mapping code.
///
/// A `Vmo` represents a contiguous byte range backed by a sparse array of
/// [`CachePage`]s. Pages are materialized on demand: anonymous VMOs allocate
/// zero-filled RAM pages, while disk-backed VMOs consult a
/// [`PageCacheBackend`] to populate or write back individual pages.
///
/// `Vmo` is the storage engine underneath [`crate::page_cache::PageCache`]. It
/// understands page indices, per-page state transitions, and optional backend
/// I/O, but it deliberately stays below filesystem policy. In particular, it
/// does not define logical EOF semantics, extent mapping policy, or whole-file
/// synchronization.
///
/// # Backing Modes
///
/// - **Anonymous VMO.** RAM-backed and zero-filled on first access. This is
///   used for anonymous memory and in-memory page caches.
/// - **Disk-backed VMO.** Connected to a [`PageCacheBackend`]. Pages are loaded
///   lazily and can later be written back.
///
/// # What `Vmo` Owns
///
/// - the page-aligned byte span returned by [`Vmo::size`];
/// - the sparse page array and page-commit path;
/// - per-page initialization, dirtying, and writeback coordination; and
/// - writable-mapping tracking for callers that opt into it.
///
/// # What Stays Outside `Vmo`
///
/// Filesystems and higher layers still own:
///
/// - logical file size and inode metadata;
/// - extent or block mapping decisions embodied by the backend; and
/// - the higher-level locking that excludes conflicting buffered I/O,
///   invalidation, truncate, or page-fault operations.
///
/// # Size Synchronization
///
/// A file that uses [`crate::page_cache::PageCache`] has two related sizes:
///
/// - the logical file size stored in filesystem metadata; and
/// - the page-aligned accessible range returned by [`Vmo::size`].
///
/// The logical file size defines EOF for filesystem operations. [`Vmo::size`]
/// defines how far lower-level VMO operations such as reads, page commits, and
/// page faults may access, and it may be larger because it is rounded up to
/// whole pages.
///
/// These two sizes must be updated while holding the same higher-level
/// inode- or file-level lock that serializes resize against buffered I/O, page
/// faults, and invalidation. The filesystem-facing resize API,
/// [`crate::page_cache::PageCache::resize`], documents the required ordering
/// between logical file size updates and [`Vmo::size`] changes.
///
/// # Disk-backed Page Lifecycle
///
/// For disk-backed pages, the state machine is:
///
/// - `Uninit -> UpToDate`: the page is populated from the backend, or a hole is
///   materialized as zeros.
/// - `Uninit -> PendingWrite -> Dirty`: a full-page overwrite reserves a page
///   before the writer has copied its new contents.
/// - `UpToDate -> Dirty`: buffered writes modify the page under the page lock.
/// - `Dirty -> UpToDate`: the VMO writeback path hands the page to the backend.
///
/// The auxiliary `is_writing_back` bit is set under the page lock, then cleared
/// later by the BIO completion callback after the writeback state has been
/// handed off. Anonymous VMOs stay `UpToDate` in steady state once a page is
/// committed.
pub struct Vmo {
    /// The backend that provides disk I/O operations, if any.
    //
    // TODO: Using `Weak` here is to avoid circular references in exfat file systems.
    // We should avoid the circular reference by design, and then we can change this to `Arc`.
    pub(super) backend: Option<Weak<dyn PageCacheBackend>>,
    /// Flags
    pub(super) flags: VmoFlags,
    /// The virtual pages where the VMO resides.
    pub(super) pages: XArray<CachePage>,
    /// The size of the VMO.
    ///
    /// This is the page-aligned VMO size, which may differ from the
    /// filesystem's logical file size. Updates must happen after locking the
    /// [`XArray`] in `pages`, so a size read after taking that lock observes
    /// the latest resize performed under the required higher-level file-size
    /// synchronization.
    pub(super) size: AtomicUsize,
    /// The status of writable mappings of the VMO.
    //
    // TODO: This field is used only by VMOs belonging to memfd (i.e., `MemfdInode`). But VMOs do
    // not have the knowledge to determine if they belong to memfd. We may want to enhance
    // `VmoOptions` to make VMOs aware of whether its writable mappings should be tracked.
    pub(super) writable_mapping_status: WritableMappingStatus,
}

impl Debug for Vmo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Vmo")
            .field("has_backend", &self.backend.is_some())
            .field("flags", &self.flags)
            .field("size", &self.size)
            .field("writable_mapping_status", &self.writable_mapping_status)
            .finish_non_exhaustive()
    }
}

bitflags! {
    /// VMO flags.
    pub struct VmoFlags: u32 {
        /// Set this flag if a VMO is resizable.
        const RESIZABLE  = 1 << 0;
        /// Set this flags if a VMO is backed by physically contiguous memory
        /// pages.
        ///
        /// To ensure the memory pages to be contiguous, these pages
        /// are allocated upon the creation of the VMO, rather than on demands.
        const CONTIGUOUS = 1 << 1;
        /// Set this flag if a VMO is backed by memory pages that supports
        /// Direct Memory Access (DMA) by devices.
        const DMA        = 1 << 2;
    }
}

/// The error type used for commit operations of [`Vmo`].
#[derive(Debug)]
pub enum VmoCommitError {
    /// A general error occurred during the commit operation.
    Err(Error),
    /// The commit operation requires an I/O operation to read the page
    /// from the backend.
    ///
    /// The wrapped value is the page index.
    NeedIo(usize),
    /// The page exists but is not yet initialized.
    ///
    /// The caller should wait for initialization to complete.
    /// Contains the page index and the page.
    WaitUntilInit(usize, CachePage),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitMode {
    Read,
    Overwrite,
}

impl CommitMode {
    fn skips_backend_read(self) -> bool {
        matches!(self, Self::Overwrite)
    }
}

impl From<Error> for VmoCommitError {
    fn from(e: Error) -> Self {
        VmoCommitError::Err(e)
    }
}

impl From<ostd::Error> for VmoCommitError {
    fn from(e: ostd::Error) -> Self {
        Error::from(e).into()
    }
}

impl Vmo {
    /// Commits the page at the given page index.
    ///
    /// For anonymous VMOs the page is zero-filled on first access.
    /// For disk-backed VMOs this may perform synchronous I/O.
    pub fn commit_on(&self, page_idx: usize) -> Result<CachePage> {
        self.commit_on_internal(page_idx, CommitMode::Read)
    }

    fn commit_on_internal(&self, page_idx: usize, commit_mode: CommitMode) -> Result<CachePage> {
        let mut locked_pages = self.pages.lock();
        if page_idx >= self.size().div_ceil(PAGE_SIZE) {
            return_errno_with_message!(Errno::EINVAL, "the offset is outside the VMO");
        }

        if let Some(disk_backed) = self.as_disk_backed() {
            disk_backed.commit_on(locked_pages, page_idx, commit_mode)
        } else {
            let mut cursor = locked_pages.cursor_mut(page_idx as u64);
            if let Some(page) = cursor.load() {
                return Ok(page.clone());
            }

            let new_page = CachePage::alloc_zero()?;
            cursor.store(new_page.clone());

            Ok(new_page)
        }
    }

    /// Commits the page corresponding to the target offset in the VMO.
    ///
    /// If the commit operation needs to perform I/O, it will return a
    /// [`VmoCommitError::NeedIo`].
    pub fn try_commit_page(
        &self,
        offset: usize,
    ) -> core::result::Result<CachePage, VmoCommitError> {
        let page_idx = offset / PAGE_SIZE;
        if offset >= self.size() {
            return Err(VmoCommitError::Err(Error::with_message(
                Errno::EINVAL,
                "the offset is outside the VMO",
            )));
        }

        let guard = disable_preempt();
        let mut cursor = self.pages.cursor(&guard, page_idx as u64);
        self.try_commit_with_cursor(&mut cursor, CommitMode::Read)
            .map(|(_, page)| page)
    }

    fn try_commit_with_cursor(
        &self,
        cursor: &mut Cursor<'_, CachePage>,
        commit_mode: CommitMode,
    ) -> core::result::Result<(usize, CachePage), VmoCommitError> {
        if let Some(disk_backed) = self.as_disk_backed() {
            if let Some((index, page)) = disk_backed.try_commit_with_cursor(cursor, commit_mode)? {
                return Ok((index, page));
            }
        } else if let Some(page) = cursor.load() {
            let index = cursor.index() as usize;
            return Ok((index, page.clone()));
        }

        // Need to commit. Only Anonymous VMOs can reach here, because disk-backed VMOs will return
        // `Err` if the page is not loaded.
        let index = cursor.index() as usize;
        Ok((index, self.commit_on_internal(index, commit_mode)?))
    }

    /// Traverses the indices within a specified range of a VMO sequentially.
    ///
    /// For each index position, you have the option to commit the page as well as
    /// perform other operations.
    ///
    /// Once a commit operation needs to perform I/O, it will return a
    /// [`VmoCommitError::NeedIo`].
    pub fn try_operate_on_range<F>(
        &self,
        range: &Range<usize>,
        operate: F,
    ) -> core::result::Result<(), VmoCommitError>
    where
        F: FnMut(
            &mut dyn FnMut() -> core::result::Result<(usize, CachePage), VmoCommitError>,
        ) -> core::result::Result<(), VmoCommitError>,
    {
        self.try_operate_on_range_internal(range, operate, CommitMode::Read)
    }

    fn try_operate_on_range_internal<F>(
        &self,
        range: &Range<usize>,
        mut operate: F,
        commit_mode: CommitMode,
    ) -> core::result::Result<(), VmoCommitError>
    where
        F: FnMut(
            &mut dyn FnMut() -> core::result::Result<(usize, CachePage), VmoCommitError>,
        ) -> core::result::Result<(), VmoCommitError>,
    {
        if range.end > self.size() {
            return Err(VmoCommitError::Err(Error::with_message(
                Errno::EINVAL,
                "operated range exceeds the vmo size",
            )));
        }

        let page_idx_range = get_page_idx_range(range);
        let guard = disable_preempt();
        let mut cursor = self.pages.cursor(&guard, page_idx_range.start as u64);
        for _ in page_idx_range {
            let mut commit_fn = || self.try_commit_with_cursor(&mut cursor, commit_mode);
            operate(&mut commit_fn)?;
            cursor.next();
        }
        Ok(())
    }

    /// Returns the current page-aligned size of the VMO in bytes.
    ///
    /// Higher layers may keep a smaller logical EOF alongside this capacity.
    pub fn size(&self) -> usize {
        self.size.load(Ordering::Acquire)
    }

    /// Returns the status of writable mappings of the VMO.
    pub fn writable_mapping_status(&self) -> &WritableMappingStatus {
        // Currently, only VMOs used by `MemfdInode` (anonymous) track writable mapping status.
        // Disk-backed VMOs do not use this field.
        debug_assert!(!self.is_disk_backed());
        &self.writable_mapping_status
    }

    pub(super) fn decommit_pages(
        &self,
        mut locked_pages: LockedXArray<CachePage>,
        range: Range<usize>,
    ) -> Result<()> {
        let page_idx_range = get_page_idx_range(&range);
        let mut cursor = locked_pages.cursor_mut(page_idx_range.start as u64);

        loop {
            cursor.remove();
            let page_idx = cursor.next_present();
            if page_idx.is_none_or(|idx| idx >= page_idx_range.end as u64) {
                break;
            }
        }

        Ok(())
    }

    /// Converts this VMO to a disk-backed VMO wrapper if it has a backend.
    ///
    /// Returns `None` if this is an anonymous VMO.
    pub(super) fn as_disk_backed(&self) -> Option<DiskBackedVmo<'_>> {
        self.backend.as_ref().and_then(|weak_backend| {
            weak_backend
                .upgrade()
                .map(|backend| DiskBackedVmo { vmo: self, backend })
        })
    }

    /// Returns whether this VMO is disk-backed.
    fn is_disk_backed(&self) -> bool {
        self.backend.is_some()
    }
}

impl Vmo {
    /// Handles a commit error by performing the necessary I/O or initialization.
    ///
    /// Returns the page index that was recovered.
    fn handle_commit_error(&self, err: VmoCommitError, commit_mode: CommitMode) -> Result<usize> {
        match err {
            VmoCommitError::Err(e) => Err(e),
            VmoCommitError::NeedIo(index) => {
                self.commit_on_internal(index, commit_mode)?;
                Ok(index)
            }
            VmoCommitError::WaitUntilInit(index, cache_page) => {
                cache_page.ensure_init(|locked_page| {
                    self.as_disk_backed()
                        .unwrap()
                        .backend
                        .read_page(index, locked_page)
                })?;
                Ok(index)
            }
        }
    }

    /// Collects a batch of committed pages from `start_idx` to `end_idx`.
    ///
    /// Returns up to `PAGE_BATCH_CAPACITY` consecutive committed pages. The
    /// caller receives only successfully committed pages and never needs to
    /// deal with [`VmoCommitError`] directly.
    ///
    /// This helper keeps the returned pages alive, but it does not pin them in
    /// the `XArray`. Callers must still serialize against `evict_range()` /
    /// `invalidate_range()` with a higher-level invalidation lock before they
    /// take per-page locks, otherwise buffered writes may dirty a page that has
    /// already been detached from the page cache.
    fn collect_pages(
        &self,
        start_idx: usize,
        end_idx: usize,
        commit_mode: CommitMode,
        pages: &mut Vec<(usize, CachePage)>,
    ) -> Result<()> {
        /// Maximum number of pages collected per batch from the `XArray`.
        const PAGE_BATCH_CAPACITY: usize = 32;

        let end_idx = end_idx.min(start_idx + PAGE_BATCH_CAPACITY);
        pages.clear();
        pages.reserve(end_idx - start_idx);
        let range = (start_idx * PAGE_SIZE)..(end_idx * PAGE_SIZE);
        let mut current_range = range.clone();

        let mut operate = |commit_fn: &mut dyn FnMut() -> core::result::Result<
            (usize, CachePage),
            VmoCommitError,
        >| {
            let (idx, page) = commit_fn()?;
            pages.push((idx, page));
            Ok(())
        };

        'retry: loop {
            match self.try_operate_on_range_internal(&current_range, &mut operate, commit_mode) {
                Ok(()) => break 'retry,
                Err(err) => {
                    let idx = self.handle_commit_error(err, commit_mode)?;
                    current_range.start = idx * PAGE_SIZE;
                }
            }
        }

        Ok(())
    }

    /// Reads data from the VMO at `offset` into `writer`.
    pub fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        let read_len = writer.avail().min(self.size().saturating_sub(offset));
        if read_len == 0 {
            return Ok(());
        }

        let range = offset..(offset + read_len);
        let page_idx_range = get_page_idx_range(&range);
        let mut current_idx = page_idx_range.start;
        let mut page_offset = offset % PAGE_SIZE;
        let mut page_batch = Vec::with_capacity(32);

        while current_idx < page_idx_range.end {
            self.collect_pages(
                current_idx,
                page_idx_range.end,
                CommitMode::Read,
                &mut page_batch,
            )?;

            for (_, page) in &page_batch {
                page.reader()
                    .skip(page_offset)
                    .read_fallible(writer)
                    .map_err(|e| Error::from(e.0))?;
                page_offset = 0;
            }

            // `current_idx < page_idx_range.end` guarantees at least one page is successfully collected here.
            current_idx = page_batch.last().unwrap().0 + 1;
        }

        Ok(())
    }

    /// Writes data from `reader` into the VMO at `offset`.
    pub fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        let write_len = reader.remain();
        if write_len == 0 {
            return Ok(());
        }

        let write_range = offset..(offset + write_len);
        let mut page_offset = offset % PAGE_SIZE;
        let mut page_batch = Vec::with_capacity(32);

        if !self.is_disk_backed() {
            return self.write_anonymous_pages(
                &write_range,
                &mut page_offset,
                reader,
                &mut page_batch,
            );
        }

        // Disk-backed VMOs require dirty tracking and may skip backend reads
        // for page-aligned ranges that will be entirely overwritten.
        if write_range.len() < PAGE_SIZE {
            return self.write_disk_backed_pages(
                &write_range,
                &mut page_offset,
                reader,
                CommitMode::Read,
                &mut page_batch,
            );
        }

        // Split into head (unaligned), middle (aligned), and tail (unaligned).
        let up_align_start = write_range.start.align_up(PAGE_SIZE);
        let down_align_end = write_range.end.align_down(PAGE_SIZE);

        if write_range.start != up_align_start {
            let head = write_range.start..up_align_start;
            self.write_disk_backed_pages(
                &head,
                &mut page_offset,
                reader,
                CommitMode::Read,
                &mut page_batch,
            )?;
        }
        if up_align_start != down_align_end {
            let mid = up_align_start..down_align_end;
            self.write_disk_backed_pages(
                &mid,
                &mut page_offset,
                reader,
                CommitMode::Overwrite,
                &mut page_batch,
            )?;
        }
        if down_align_end != write_range.end {
            let tail = down_align_end..write_range.end;
            self.write_disk_backed_pages(
                &tail,
                &mut page_offset,
                reader,
                CommitMode::Read,
                &mut page_batch,
            )?;
        }

        Ok(())
    }

    /// Writes data to pages without dirty tracking.
    ///
    /// Used for anonymous VMO writes where pages are always in the `UpToDate`
    /// state.
    fn write_anonymous_pages(
        &self,
        range: &Range<usize>,
        page_offset: &mut usize,
        reader: &mut VmReader,
        page_batch: &mut Vec<(usize, CachePage)>,
    ) -> Result<()> {
        let page_idx_range = get_page_idx_range(range);
        let mut current_idx = page_idx_range.start;

        while current_idx < page_idx_range.end {
            self.collect_pages(
                current_idx,
                page_idx_range.end,
                CommitMode::Read,
                page_batch,
            )?;

            for (_, page) in page_batch.iter() {
                page.writer()
                    .skip(*page_offset)
                    .write_fallible(reader)
                    .map_err(|e| Error::from(e.0))?;
                *page_offset = 0;
            }

            // `current_idx < page_idx_range.end` guarantees at least one page is successfully collected here.
            current_idx = page_batch.last().unwrap().0 + 1;
        }

        Ok(())
    }

    /// Writes data to disk-backed pages with dirty tracking.
    ///
    /// Each page is locked before writing to ensure correct state transitions
    /// and marked dirty for later writeback.
    fn write_disk_backed_pages(
        &self,
        range: &Range<usize>,
        page_offset: &mut usize,
        reader: &mut VmReader,
        commit_mode: CommitMode,
        page_batch: &mut Vec<(usize, CachePage)>,
    ) -> Result<()> {
        let page_idx_range = get_page_idx_range(range);
        let mut current_idx = page_idx_range.start;

        while current_idx < page_idx_range.end {
            self.collect_pages(current_idx, page_idx_range.end, commit_mode, page_batch)?;

            // `current_idx < page_idx_range.end` guarantees at least one page is successfully collected here.
            let next_idx = page_batch.last().unwrap().0 + 1;

            for (_, page) in page_batch.iter() {
                let locked_page = page.clone().lock();
                let written_size = match locked_page
                    .writer()
                    .skip(*page_offset)
                    .write_fallible(reader)
                {
                    Ok(written_size) => written_size,
                    Err((err, written_size)) => {
                        if written_size > 0 {
                            locked_page.set_dirty();
                        } else if locked_page.is_pending_write() {
                            // If the page is pending write and the write fails without
                            // writing any byte, we should clear the pending write state
                            // to avoid blocking other operations on this page.
                            locked_page.clear_pending_write();
                        }
                        return Err(Error::from(err));
                    }
                };

                if written_size > 0 {
                    locked_page.set_dirty();
                }

                *page_offset = 0;
            }

            current_idx = next_idx;
        }

        Ok(())
    }
}

impl VmIo for Vmo {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> ostd::Result<()> {
        self.read(offset, writer)?;
        Ok(())
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> ostd::Result<()> {
        self.write(offset, reader)?;
        Ok(())
    }
}

impl VmIoFill for Vmo {
    fn fill_zeros(
        &self,
        offset: usize,
        len: usize,
    ) -> core::result::Result<(), (ostd::Error, usize)> {
        // TODO: Support efficient `fill_zeros()`.
        for i in 0..len {
            match self.write_slice(offset + i, &[0u8]) {
                Ok(()) => continue,
                Err(err) => return Err((err, i)),
            }
        }
        Ok(())
    }
}

/// A wrapper around a disk-backed VMO that provides specialized operations.
///
/// This structure is created by calling [`Vmo::as_disk_backed()`] and provides
/// access to disk-backed specific functionality like reading from storage and
/// managing dirty pages.
pub struct DiskBackedVmo<'a> {
    vmo: &'a Vmo,
    backend: Arc<dyn PageCacheBackend>,
}

impl<'a> DiskBackedVmo<'a> {
    /// Commits a page at the given index for a disk-backed VMO.
    fn commit_on(
        &self,
        mut locked_pages: LockedXArray<'_, CachePage>,
        page_idx: usize,
        commit_mode: CommitMode,
    ) -> Result<CachePage> {
        let mut cursor = locked_pages.cursor_mut(page_idx as u64);
        if let Some(page) = cursor.load() {
            let page = page.clone();
            if self.backend.npages() > page_idx {
                drop(locked_pages);
                if !commit_mode.skips_backend_read() {
                    page.ensure_init(|locked_page| self.backend.read_page(page_idx, locked_page))?;
                }
            }

            return Ok(page);
        };

        // Page is within the file bounds - need to read from backend
        if self.backend.npages() > page_idx {
            let new_page = CachePage::alloc_uninit()?;
            // Acquiring the lock from a new page must succeed.
            let locked_page = new_page.try_lock().unwrap();
            if commit_mode.skips_backend_read() {
                locked_page.set_pending_write();
            }

            cursor.store(locked_page.clone());

            drop(locked_pages);

            if commit_mode.skips_backend_read() {
                // Page will be completely overwritten, no need to read
                Ok(locked_page.unlock())
            } else {
                // Read the page from backend storage
                self.backend.read_page(page_idx, locked_page)?;
                Ok(new_page)
            }
        } else {
            // Page is beyond file bounds - treat as hole (zero-filled)
            let new_page = CachePage::alloc_zero()?;
            cursor.store(new_page.clone());

            Ok(new_page)
        }
    }

    /// Attempts to commit a page using a cursor, without blocking on I/O.
    fn try_commit_with_cursor(
        &self,
        cursor: &mut Cursor<'_, CachePage>,
        commit_mode: CommitMode,
    ) -> core::result::Result<Option<(usize, CachePage)>, VmoCommitError> {
        let page_idx = cursor.index() as usize;

        let Some(page) = cursor.load() else {
            return Err(VmoCommitError::NeedIo(page_idx));
        };

        // If page is within file bounds, check if it's initialized
        if !commit_mode.skips_backend_read() && self.backend.npages() > page_idx && page.is_uninit()
        {
            return Err(VmoCommitError::WaitUntilInit(page_idx, page.clone()));
        }

        Ok(Some((page_idx, page.clone())))
    }

    /// Writes a dirty page to the backend asynchronously.
    ///
    /// The VMO owns the writeback state transitions so backend overrides of
    /// `PageCacheBackend::write_page_async` only need to submit the I/O.
    fn write_page_async(&self, idx: usize, locked_page: LockedCachePage) -> Result<BioWaiter> {
        locked_page.wait_until_finish_writing_back();
        locked_page.set_writing_back();
        locked_page.set_up_to_date();

        let page = locked_page.clone();
        let res = self.backend.write_page_async(idx, locked_page);

        if res.is_err() {
            let locked_page = page.lock();
            locked_page.set_dirty();
            clear_writing_back(locked_page.deref());
        }

        res
    }

    /// Collects dirty pages in the specified byte range.
    ///
    /// The pages are only read (not removed) from the XArray.
    fn collect_dirty_pages(&self, range: &Range<usize>) -> Vec<(usize, CachePage)> {
        let locked_pages = self.vmo.pages.lock();
        if range.start > self.size() {
            return Vec::new();
        }

        let page_idx_range = get_page_idx_range(range);
        let npages = self.backend.npages();
        if page_idx_range.start >= npages {
            return Vec::new();
        }

        let mut dirty_pages = Vec::new();

        let mut cursor = locked_pages.cursor(page_idx_range.start as u64);
        if let Some(page) = cursor.load()
            && page.is_dirty()
        {
            dirty_pages.push((page_idx_range.start, page.clone()));
        }

        while let Some(page_idx) = cursor.next_present() {
            let page_idx = page_idx as usize;
            if page_idx >= page_idx_range.end || page_idx >= npages {
                break;
            }

            let page = cursor.load().unwrap();
            if page.is_dirty() {
                dirty_pages.push((page_idx, page.clone()));
            }
        }

        dirty_pages
    }

    /// Writes back dirty pages in the specified byte range to the backend storage.
    pub(super) fn flush_dirty_pages(&self, range: &Range<usize>) -> Result<()> {
        let mut waiter = BioWaiter::new();

        let dirty_pages = self.collect_dirty_pages(range);
        for (idx, page) in dirty_pages {
            let locked_page = page.lock();
            waiter.concat(self.write_page_async(idx, locked_page)?);
        }

        if waiter.wait() != Some(BioStatus::Complete) {
            return_errno!(Errno::EIO);
        }

        Ok(())
    }

    /// Removes up-to-date (clean) pages in the specified byte range from the page cache.
    ///
    /// Only pages in the `UpToDate` state are removed. Dirty and uninitialized
    /// pages are left in place.
    //
    // TODO: Returns `Err` if any up-to-date page has been mapped.
    // TODO: Integrate reverse mappings or an explicit invalidation lock so this
    // path can coordinate with page faults.
    pub(super) fn evict_up_to_date_pages(&self, range: &Range<usize>) -> Result<()> {
        let mut locked_pages = self.vmo.pages.lock();
        if range.start > self.size() {
            return Ok(());
        }

        let page_idx_range = get_page_idx_range(range);
        let npages = self.backend.npages();
        if page_idx_range.start >= npages {
            return Ok(());
        }

        let mut cursor = locked_pages.cursor_mut(page_idx_range.start as u64);

        if cursor.load().is_some_and(|p| p.is_up_to_date()) {
            cursor.remove();
        }

        while let Some(page_idx) = cursor.next_present() {
            let page_idx = page_idx as usize;
            if page_idx >= page_idx_range.end || page_idx >= npages {
                break;
            }

            if cursor.load().is_some_and(|p| p.is_up_to_date()) {
                cursor.remove();
            }
        }

        Ok(())
    }
}

impl Deref for DiskBackedVmo<'_> {
    type Target = Vmo;

    fn deref(&self) -> &Self::Target {
        self.vmo
    }
}

/// Gets the page index range that contains the offset range of VMO.
pub fn get_page_idx_range(vmo_offset_range: &Range<usize>) -> Range<usize> {
    let start = vmo_offset_range.start.align_down(PAGE_SIZE);
    let end = vmo_offset_range.end.align_up(PAGE_SIZE);
    (start / PAGE_SIZE)..(end / PAGE_SIZE)
}

/// The status of writable mappings of a VMO, i.e., shared mappings that may
/// include the `PROT_WRITE` permission.
///
/// Internally, it uses an `AtomicIsize` counter with the following rules:
///
/// - **Positive values**: number of active writable mappings.
/// - **Zero**: no writable mappings, and writable mappings are still allowed.
/// - **Negative values**: writable mappings are denied.
#[derive(Debug, Default)]
pub struct WritableMappingStatus(AtomicIsize);

impl WritableMappingStatus {
    /// Builds a new writable mapping.
    ///
    /// Fails with `EPERM` if writable mappings have been denied.
    pub fn map(&self) -> Result<()> {
        // Increase unless negative
        self.0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                (v >= 0).then(|| v + 1)
            })
            .map_err(|_| Error::with_message(Errno::EPERM, "writable mappings have been denied"))?;
        Ok(())
    }

    /// Denies any future writable mapping.
    ///
    /// Fails with `EBUSY` if there are still active writable mappings.
    pub fn deny(&self) -> Result<()> {
        // Decrease unless positive
        self.0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                (v <= 0).then_some(-1)
            })
            .map_err(|_| {
                Error::with_message(Errno::EBUSY, "there are still active writable mappings")
            })?;
        Ok(())
    }

    /// Increments the writable mapping counter.
    ///
    /// Typically used when splitting an existing mapping, or forking a process.
    pub fn increment(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrements the writable mapping counter.
    ///
    /// Typically used when unmapping a region, exiting a process, or merging mappings.
    pub fn decrement(&self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
