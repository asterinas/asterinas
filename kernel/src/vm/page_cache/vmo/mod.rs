// SPDX-License-Identifier: MPL-2.0

//! Lower-level virtual memory object machinery used by the page cache.
//!
//! This submodule defines [`Vmo`] and [`VmoOptions`], the generic memory-object
//! layer underneath [`crate::vm::page_cache::PageCache`]. A [`Vmo`] owns a sparse
//! array of pages, commits them on demand, and optionally cooperates with a
//! [`crate::vm::page_cache::PageCacheBackend`] to populate or write back pages with
//! a backend.
//!
//! # When to Use This Module
//!
//! - Filesystem buffered I/O, invalidation, and resize should usually stay at
//!   [`crate::vm::page_cache::PageCache`].
//! - Memory-mapping, page-fault, and other lower-level VM paths that need
//!   direct page-oriented access should work with [`Vmo`].

use core::{
    cmp::min,
    ops::{Deref, Range},
    sync::atomic::{AtomicIsize, AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use io_util::batch::IoBatch;
use ostd::{
    mm::{HasPaddr, io::util::HasVmReaderWriter},
    task::disable_preempt,
};
use xarray::{Cursor, LockedXArray, XArray};

use crate::{
    prelude::*,
    vm::page_cache::{CachePage, CachePageExt, PageCacheBackend},
};

mod options;

pub use options::VmoOptions;

/// Page-indexed memory object used by the page cache and mapping code.
///
/// A `Vmo` represents a contiguous byte range backed by a sparse array of
/// [`CachePage`]s. Pages are materialized on demand: anonymous VMOs allocate
/// zero-filled RAM pages, while VMOs with a backend consult a
/// [`PageCacheBackend`] to populate or write back individual pages.
///
/// `Vmo` is the storage engine underneath [`crate::vm::page_cache::PageCache`]. It
/// understands page indices, per-page state transitions, and optional backend
/// I/O, but it deliberately stays below filesystem policy. In particular, it
/// does not define logical EOF semantics, extent mapping policy, or whole-file
/// synchronization.
///
/// # Backing Modes
///
/// - **Anonymous VMO.** RAM-backed and zero-filled on first access. This is
///   used for anonymous memory and in-memory page caches.
/// - **Backend VMO.** Connected to a [`PageCacheBackend`]. Pages are loaded
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
/// A file that uses [`crate::vm::page_cache::PageCache`] has two related sizes:
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
/// [`crate::vm::page_cache::PageCache::resize`], documents the required ordering
/// between logical file size updates and [`Vmo::size`] changes.
///
/// # Backend Page Lifecycle
///
/// For pages with a backend, the state machine is:
///
/// - `Uninit -> UpToDate`: the page is populated from the backend, or a hole is
///   materialized as zeros.
/// - `Uninit -> Dirty`: a full-page overwrite commits and keeps the page
///   locked until the writer has copied its new contents.
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
    /// The commit operation requires backend I/O for the page at `index`.
    NeedIo { index: usize },
    /// The page exists but is not yet initialized.
    ///
    /// The caller should wait for initialization of `page` at `index` to
    /// complete.
    WaitUntilInit { index: usize, page: CachePage },
}

impl VmoCommitError {
    /// Returns the page index whose commit is pending on I/O or initialization.
    pub fn pending_index(&self) -> Result<usize> {
        match self {
            Self::NeedIo { index } | Self::WaitUntilInit { index, .. } => Ok(*index),
            Self::Err(e) => Err(*e),
        }
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

impl Vmo {
    /// Commits the page at `page_idx`, blocking on backend I/O if required.
    ///
    /// For anonymous VMOs the page is zero-filled on first access.
    /// For VMOs with a backend this may perform synchronous I/O.
    pub fn commit_on(&self, page_idx: usize) -> Result<CachePage> {
        self.commit_on_internal(page_idx, CommitMode::Read)
    }

    fn commit_on_internal(&self, page_idx: usize, commit_mode: CommitMode) -> Result<CachePage> {
        if let Some(backed_vmo) = self.as_backed_vmo() {
            return backed_vmo.commit_on_internal(page_idx, commit_mode);
        }

        self.commit_on_anonymous(page_idx)
    }

    fn commit_on_anonymous(&self, page_idx: usize) -> Result<CachePage> {
        let mut locked_pages = self.pages.lock();
        if page_idx >= self.size().div_ceil(PAGE_SIZE) {
            return_errno_with_message!(Errno::EINVAL, "the offset is outside the VMO");
        }

        let mut cursor = locked_pages.cursor_mut(page_idx as u64);
        if let Some(page) = cursor.load() {
            return Ok(page.clone());
        }

        let new_page = CachePage::alloc_zero()?;
        cursor.store(new_page.clone());
        Ok(new_page)
    }

    /// Tries to commit the page covering byte `offset` without blocking on I/O.
    ///
    /// If the commit operation needs to perform I/O, it will return a
    /// [`VmoCommitError::NeedIo`].
    pub fn try_commit_page(&self, offset: usize) -> Result<CachePage, VmoCommitError> {
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
    ) -> Result<(usize, CachePage), VmoCommitError> {
        if let Some(backed_vmo) = self.as_backed_vmo() {
            return backed_vmo.try_commit_with_cursor(cursor, commit_mode);
        } else if let Some(page) = cursor.load() {
            let index = cursor.index() as usize;
            return Ok((index, page.clone()));
        }

        // Need to commit. Only Anonymous VMOs can reach here, because VMOs
        // with a backend will return `Err` if the page is not loaded.
        let index = cursor.index() as usize;
        let page = self.commit_on_anonymous(index)?;
        Ok((index, page))
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
    ) -> Result<(), VmoCommitError>
    where
        F: FnMut(
            &mut dyn FnMut() -> Result<(usize, CachePage), VmoCommitError>,
        ) -> Result<(), VmoCommitError>,
    {
        self.try_operate_on_range_internal(range, operate, CommitMode::Read)
    }

    fn try_operate_on_range_internal<F>(
        &self,
        range: &Range<usize>,
        mut operate: F,
        commit_mode: CommitMode,
    ) -> Result<(), VmoCommitError>
    where
        F: FnMut(
            &mut dyn FnMut() -> Result<(usize, CachePage), VmoCommitError>,
        ) -> Result<(), VmoCommitError>,
    {
        if range.end > self.size() {
            return Err(VmoCommitError::Err(Error::with_message(
                Errno::EINVAL,
                "the range to operate exceeds the VMO size",
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
        // VMOs with a backend do not use this field.
        debug_assert!(!self.has_backend());
        &self.writable_mapping_status
    }

    /// Decommits anonymous pages in the specified byte range.
    pub(super) fn decommit_anon_pages(
        &self,
        mut locked_pages: LockedXArray<CachePage>,
        range: Range<usize>,
    ) -> Result<()> {
        debug_assert!(!self.has_backend());

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

    /// Converts this VMO to a backend VMO wrapper if it has a backend.
    ///
    /// Returns `None` if this is an anonymous VMO.
    pub(super) fn as_backed_vmo(&self) -> Option<BackedVmo<'_>> {
        self.backend.as_ref().and_then(|weak_backend| {
            weak_backend
                .upgrade()
                .map(|backend| BackedVmo { vmo: self, backend })
        })
    }

    /// Returns whether this VMO has a backend.
    fn has_backend(&self) -> bool {
        self.backend.is_some()
    }
}

// Implement the read/write methods for `Vmo`.
impl Vmo {
    /// Maximum number of pages collected per batch from the `XArray`.
    const PAGE_BATCH_CAPACITY: usize = 32;

    /// Reads data from the VMO at `offset` into `writer`.
    pub fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        let read_len = writer.avail().min(self.size().saturating_sub(offset));
        if read_len == 0 {
            return Ok(());
        }
        let read_end = offset + read_len;

        let range = offset..read_end;
        let page_idx_range = get_page_idx_range(&range);
        let mut current_idx = page_idx_range.start;
        let mut page_offset = offset % PAGE_SIZE;
        let mut page_batch =
            Vec::with_capacity(min(page_idx_range.len(), Self::PAGE_BATCH_CAPACITY));

        while current_idx < page_idx_range.end {
            self.collect_pages(
                current_idx,
                page_idx_range.end,
                CommitMode::Read,
                &mut page_batch,
            )?;

            for (_, page) in page_batch.iter() {
                page.reader().skip(page_offset).read_fallible(writer)?;
                page_offset = 0;
            }

            // `current_idx < page_idx_range.end` guarantees at least one page is successfully collected here.
            current_idx = page_batch.last().unwrap().0 + 1;
        }

        Ok(())
    }

    /// Writes data from `reader` into the VMO at `offset`.
    pub fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        let write_len = reader.remain().min(self.size().saturating_sub(offset));
        if write_len == 0 {
            return Ok(());
        }
        let write_end = offset + write_len;

        let write_range = offset..write_end;
        let mut page_offset = offset % PAGE_SIZE;
        let mut page_batch = Vec::with_capacity(min(
            write_range.len().div_ceil(PAGE_SIZE),
            Self::PAGE_BATCH_CAPACITY,
        ));

        if !self.has_backend() {
            return self.write_anonymous_pages(
                &write_range,
                &mut page_offset,
                reader,
                &mut page_batch,
            );
        }

        // VMOs with a backend require dirty tracking and may skip backend reads
        // for page-aligned ranges that will be entirely overwritten.
        if write_range.len() < PAGE_SIZE {
            return self.write_pages_with_backend(
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
            self.write_pages_with_backend(
                &head,
                &mut page_offset,
                reader,
                CommitMode::Read,
                &mut page_batch,
            )?;
        }
        if up_align_start != down_align_end {
            let mid = up_align_start..down_align_end;
            self.write_pages_with_backend(
                &mid,
                &mut page_offset,
                reader,
                CommitMode::Overwrite,
                &mut page_batch,
            )?;
        }
        if down_align_end != write_range.end {
            let tail = down_align_end..write_range.end;
            self.write_pages_with_backend(
                &tail,
                &mut page_offset,
                reader,
                CommitMode::Read,
                &mut page_batch,
            )?;
        }

        Ok(())
    }

    /// Fills the specified range with zeros.
    pub(super) fn fill_zeros(&self, range: Range<usize>) -> Result<()> {
        if range.is_empty() {
            return Ok(());
        }
        if range.end > self.size() {
            return_errno_with_message!(Errno::EINVAL, "the range to fill exceeds the VMO size");
        }

        static ZERO_PAGE: [u8; PAGE_SIZE] = [0; PAGE_SIZE];

        let mut current_offset = range.start;
        while current_offset < range.end {
            let page_remaining = PAGE_SIZE - current_offset % PAGE_SIZE;
            let write_len = (range.end - current_offset).min(page_remaining);
            let mut reader = VmReader::from(&ZERO_PAGE[..write_len]).to_fallible();

            self.write(current_offset, &mut reader)?;
            current_offset += write_len;
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
                page.writer().skip(*page_offset).write_fallible(reader)?;
                *page_offset = 0;
            }

            // `current_idx < page_idx_range.end` guarantees at least one page is successfully collected here.
            current_idx = page_batch.last().unwrap().0 + 1;
        }

        Ok(())
    }

    /// Writes data to pages with backend dirty tracking.
    ///
    /// Each page is locked before writing to ensure correct state transitions
    /// and marked dirty for later writeback.
    fn write_pages_with_backend(
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

            for (_, page) in page_batch.drain(..) {
                let locked_page = page.lock();

                let written_size = match locked_page
                    .writer()
                    .skip(*page_offset)
                    .write_fallible(reader)
                {
                    Ok(written_size) => written_size,
                    Err((err, written_size)) => {
                        // If the page is not initialized, keep it as it is on a partial write.
                        if written_size > 0 && locked_page.is_up_to_date() {
                            locked_page.set_dirty();
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

    /// Collects a batch of committed pages from `start_idx` to `end_idx`.
    ///
    /// Returns up to a small batch of consecutive committed pages. The
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
        let end_idx = end_idx.min(start_idx + pages.capacity());
        pages.clear();
        let range = (start_idx * PAGE_SIZE)..(end_idx * PAGE_SIZE);
        let mut current_range = range.clone();

        'retry: loop {
            let mut operate =
                |commit_fn: &mut dyn FnMut() -> Result<(usize, CachePage), VmoCommitError>| {
                    let (idx, page) = commit_fn()?;
                    pages.push((idx, page));
                    Ok(())
                };
            match self.try_operate_on_range_internal(&current_range, &mut operate, commit_mode) {
                Ok(()) => break 'retry,
                Err(err) => {
                    let (idx, page) = self.handle_commit_error(err, commit_mode)?;
                    pages.push((idx, page));
                    current_range.start = (idx + 1) * PAGE_SIZE;
                    if current_range.start >= range.end {
                        break 'retry;
                    }
                }
            }
        }

        Ok(())
    }

    /// Handles a commit error by performing the necessary I/O or initialization.
    ///
    /// Returns the page index and page that were recovered.
    fn handle_commit_error(
        &self,
        err: VmoCommitError,
        commit_mode: CommitMode,
    ) -> Result<(usize, CachePage)> {
        match err {
            VmoCommitError::Err(e) => Err(e),
            VmoCommitError::NeedIo { index } => {
                Ok((index, self.commit_on_internal(index, commit_mode)?))
            }
            VmoCommitError::WaitUntilInit { index, page } => {
                page.ensure_init(|locked_page| {
                    self.as_backed_vmo()
                        .unwrap()
                        .backend
                        .read_page(index, locked_page)
                })?;
                Ok((index, page))
            }
        }
    }
}

/// A wrapper around a VMO with a backend that provides specialized operations.
///
/// This structure is created by calling [`Vmo::as_backed_vmo()`] and provides
/// access to backend-specific functionality like reading from storage and
/// managing dirty pages.
pub struct BackedVmo<'a> {
    vmo: &'a Vmo,
    backend: Arc<dyn PageCacheBackend>,
}

impl<'a> BackedVmo<'a> {
    /// Writes back dirty pages in the specified byte range to the backend storage.
    pub(super) fn flush_dirty_pages(&self, range: &Range<usize>) -> Result<()> {
        let locked_pages = self.vmo.pages.lock();
        if range.start >= self.size() {
            return Ok(());
        }

        let page_idx_range = get_page_idx_range(range);
        let dirty_pages =
            self.collect_pages_if(locked_pages, page_idx_range, |_, page| page.is_dirty());

        let mut io_batch = IoBatch::with_capacity(dirty_pages.len());
        for (idx, page) in dirty_pages {
            let locked_page = page.lock();
            self.backend
                .write_page_async(idx, locked_page, &mut io_batch)?;
        }

        io_batch.wait_all().map_err(Into::into)
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
        let locked_pages = self.vmo.pages.lock();
        if range.start >= self.size() {
            return Ok(());
        }

        let page_idx_range = get_page_idx_range(range);
        let pages_to_evict =
            self.collect_pages_if(locked_pages, page_idx_range, |_, page| page.is_up_to_date());
        self.wait_for_writeback_and_remove_pages(pages_to_evict);

        Ok(())
    }

    /// Decommits pages with a backend in the specified byte range.
    ///
    /// Dirty pages in the range are discarded without being written back. If a
    /// page is already under writeback, decommit waits for that writeback to
    /// finish before removing the page from the `XArray`.
    pub(super) fn decommit_pages(
        &self,
        locked_pages: LockedXArray<CachePage>,
        range: &Range<usize>,
    ) -> Result<()> {
        // Do not check `self.size()` here. It contains the new size, however, we want to decommit
        // pages outside of the new range. See `PageCache::resize` for a concrete example.

        let page_idx_range = get_page_idx_range(range);
        let pages_to_decommit = self.collect_pages_if(locked_pages, page_idx_range, |_, _| true);
        self.wait_for_writeback_and_remove_pages(pages_to_decommit);

        Ok(())
    }

    /// Commits a page at the given index for a VMO with a backend.
    fn commit_on_internal(&self, page_idx: usize, commit_mode: CommitMode) -> Result<CachePage> {
        let mut locked_pages = self.pages.lock();
        if page_idx >= self.size().div_ceil(PAGE_SIZE) {
            return_errno_with_message!(Errno::EINVAL, "the offset is outside the VMO");
        }

        let mut cursor = locked_pages.cursor_mut(page_idx as u64);

        if let Some(page) = cursor.load() {
            let page = page.clone();
            drop(locked_pages);

            if !commit_mode.skips_backend_read() {
                page.ensure_init(|locked_page| self.backend.read_page(page_idx, locked_page))?;
                return Ok(page);
            }

            return Ok(page);
        };

        // The page is within the file bounds - need to allocate a cache page.
        let uninit_page = CachePage::alloc_uninit()?;
        cursor.store(uninit_page.clone());
        drop(locked_pages);

        if commit_mode.skips_backend_read() {
            // The page will be completely overwritten, no need to read.
            Ok(uninit_page)
        } else {
            // Read the page from the backend storage.
            uninit_page.ensure_init(|locked_page| self.backend.read_page(page_idx, locked_page))?;
            Ok(uninit_page)
        }
    }

    /// Attempts to commit a page using a cursor, without blocking on I/O.
    fn try_commit_with_cursor(
        &self,
        cursor: &mut Cursor<'_, CachePage>,
        commit_mode: CommitMode,
    ) -> Result<(usize, CachePage), VmoCommitError> {
        let page_idx = cursor.index() as usize;

        let Some(page) = cursor.load() else {
            return Err(VmoCommitError::NeedIo { index: page_idx });
        };

        // Check if the page is initialized.
        if !commit_mode.skips_backend_read() && page.is_uninit() {
            return Err(VmoCommitError::WaitUntilInit {
                index: page_idx,
                page: page.clone(),
            });
        }

        Ok((page_idx, page.clone()))
    }

    /// Collects pages in the specified page-index range that satisfy `should_collect`.
    ///
    /// The pages are only read (not removed) from the XArray.
    fn collect_pages_if<F>(
        &self,
        locked_pages: LockedXArray<CachePage>,
        page_idx_range: Range<usize>,
        mut should_collect: F,
    ) -> Vec<(usize, CachePage)>
    where
        F: FnMut(usize, &CachePage) -> bool,
    {
        if page_idx_range.is_empty() {
            return Vec::new();
        }

        let mut collected_pages = Vec::new();

        let mut cursor = locked_pages.cursor(page_idx_range.start as u64);
        if let Some(page) = cursor.load()
            && should_collect(page_idx_range.start, &page)
        {
            collected_pages.push((page_idx_range.start, page.clone()));
        }

        while let Some(page_idx) = cursor.next_present() {
            let page_idx = page_idx as usize;
            if page_idx >= page_idx_range.end {
                break;
            }

            let page = cursor.load().unwrap();
            if should_collect(page_idx, &page) {
                collected_pages.push((page_idx, page.clone()));
            }
        }

        collected_pages
    }

    /// Waits for writeback on collected pages, then removes them from the `XArray`.
    ///
    /// The pages are collected before this function is called so the `XArray`
    /// lock can be released while waiting for in-flight writeback to finish.
    /// Waiting before removal also prevents a later commit from installing a
    /// new page for the same index and starting duplicate BIOs while the old
    /// page is still under writeback.
    fn wait_for_writeback_and_remove_pages(&self, pages_to_remove: Vec<(usize, CachePage)>) {
        for (_, page) in pages_to_remove.iter() {
            let locked_page = page.lock_guard();
            locked_page.wait_until_finish_writing_back();
        }

        let mut locked_pages = self.vmo.pages.lock();

        for (page_idx, page) in pages_to_remove {
            let mut cursor = locked_pages.cursor_mut(page_idx as u64);

            // The caller will hold the higher-level lock that excludes concurrent
            // removal, read/write and page fault handling, so the collected index
            // still identifies the page that need to be deleted.
            debug_assert!(
                cursor
                    .load()
                    .is_some_and(|current_page| current_page.paddr() == page.paddr())
            );
            cursor.remove();
        }
    }
}

impl Deref for BackedVmo<'_> {
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
