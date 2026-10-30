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
    vm::{
        page_cache::{
            CachePage, CachePageExt, LockedCachePage, PageCacheBackend, PageRun, Readahead,
            cache_page::PageState, readahead::ReadaheadTrigger,
        },
        vmar::Rmap,
    },
};

mod options;

pub(crate) use options::VmoOptions;

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
/// - `UpToDate -> Dirty`: buffered writes modify the page under the page lock,
///   or the page is mapped writably to userspace.
/// - `Dirty -> UpToDate`: the VMO writeback path hands the page to the backend.
/// - `UpToDate -> Evicted`: the page is evicted and the page lock is held until
///   the page is removed from the VMO.
///
/// The auxiliary `is_writing_back` bit is set under the page lock, then cleared
/// later by the I/O completion callback after the writeback state has been
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
    /// Reverse mappings.
    pub(super) rmap: Mutex<Rmap>,
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
    pub(crate) struct VmoFlags: u32 {
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
pub(crate) enum VmoCommitError {
    /// A general error occurred during the commit operation.
    Err(Error),
    /// The commit operation requires backend I/O for the page at `index`.
    NeedIo { index: usize },
}

impl VmoCommitError {
    /// Returns the page index whose commit is pending on I/O.
    pub(crate) fn pending_index(&self) -> Result<usize> {
        match self {
            Self::NeedIo { index } => Ok(*index),
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

/// Controls whether committing a page initializes it from the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitMode {
    /// Reads uninitialized pages from the backend before returning.
    Read,
    /// Skips initialization, leaving uninitialized pages for the caller to initialize.
    SkipInit,
}

impl CommitMode {
    fn skips_init(self) -> bool {
        matches!(self, Self::SkipInit)
    }
}

/// A mode specified when committing a new [`Vmo`] page to the map in a page table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VmoMapMode {
    /// The page is private. It can be a snapshot.
    Private,
    /// The page is shared. It needs to be read.
    SharedRead,
    /// The page is shared. It needs to be read and written.
    SharedWrite,
}

impl VmoMapMode {
    /// Ensures that the page can be mapped to a page table.
    fn ensure(self, page: CachePage) {
        match page.state() {
            PageState::UpToDate if self.is_write() => {
                let locked_page = page.lock_guard();
                if locked_page.state().is_up_to_date() {
                    locked_page.set_dirty();
                }
            }
            PageState::UpToDate | PageState::Dirty => (),
            PageState::Evicted if !self.is_shared() => (),
            state @ (PageState::Uninit | PageState::Evicted) => {
                // We shouldn't reach this point if the page is not initialized. `CommitMode::Read`
                // should have been specified earlier to either read the page from the backend
                // storage or report an error if reading fails.
                debug_assert_eq!(state, PageState::Evicted);
                // The page may be evicted. It still contains valid data. However, it cannot be
                // mapped to a page table. We acquire the page lock here so that, when the caller
                // retries, it won't see this page anymore.
                let _ = page.lock_guard();
            }
        }
    }

    /// Tries to ensure that the page can be mapped to a page table without blocking.
    ///
    /// On success, returns a mode that satisfies `self` but may grant the caller more permissions
    /// when mapping `page`. See also the "Map modes" section in [`Vmo::try_commit_page`].
    fn try_ensure(self, page_idx: usize, page: &CachePage) -> Result<Self, VmoCommitError> {
        match page.state() {
            PageState::UpToDate if self.is_write() => {
                if let Some(locked_page) = page.try_lock_guard()
                    && locked_page.state().is_up_to_date()
                {
                    locked_page.set_dirty();
                    Ok(Self::SharedWrite)
                } else {
                    Err(VmoCommitError::NeedIo { index: page_idx })
                }
            }
            PageState::UpToDate => Ok(Self::SharedRead),
            PageState::Dirty => Ok(Self::SharedWrite),
            PageState::Evicted if !self.is_shared() => Ok(Self::Private),
            PageState::Uninit | PageState::Evicted => {
                Err(VmoCommitError::NeedIo { index: page_idx })
            }
        }
    }

    fn is_write(self) -> bool {
        matches!(self, Self::SharedWrite)
    }

    fn is_shared(self) -> bool {
        matches!(self, Self::SharedRead | Self::SharedWrite)
    }
}

impl Vmo {
    /// Commits the page at `page_idx`, blocking on backend I/O if required.
    ///
    /// For anonymous VMOs the page is zero-filled on first access.
    /// For VMOs with a backend this may perform synchronous I/O.
    pub(crate) fn commit_on(&self, page_idx: usize, map_mode: VmoMapMode) -> Result<()> {
        let page = self.commit_on_internal(page_idx, CommitMode::Read)?;
        map_mode.ensure(page);
        Ok(())
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
    ///
    /// If the VMO corresponds to a page cache, callers must hold the page table
    /// lock and insert the committed page atomically to the page table after
    /// this method returns.
    ///
    /// # Map modes
    ///
    /// `map_mode` specifies the requested mapping mode for the page in the page
    /// table. If successful, this method returns another [`VmoMapMode`] that
    /// grants at least the requested mode, but may grant additional permissions
    /// according to the page state. Callers can use this information to prevent
    /// some future page faults.
    ///
    /// For example, if the page is already dirty, mapping it as writable is
    /// permitted even if only read access is required in `map_mode`. In this
    /// case, the requested mode (`map_mode`) is [`VmoMapMode::SharedRead`] but
    /// the granted mode (the return value) is [`VmoMapMode::SharedWrite`].
    pub(crate) fn try_commit_page(
        &self,
        offset: usize,
        map_mode: VmoMapMode,
    ) -> Result<(CachePage, VmoMapMode), VmoCommitError> {
        let page_idx = offset / PAGE_SIZE;
        if offset >= self.size() {
            return Err(VmoCommitError::Err(Error::with_message(
                Errno::EINVAL,
                "the offset is outside the VMO",
            )));
        }

        let (_, page) = {
            let guard = disable_preempt();
            let mut cursor = self.pages.cursor(&guard, page_idx as u64);
            // `VmoMapMode::try_ensure` checks if the page has been initialized.
            self.try_commit_with_cursor(&mut cursor)?
        };
        let mode = map_mode.try_ensure(page_idx, &page)?;
        Ok((page, mode))
    }

    fn try_commit_with_cursor(
        &self,
        cursor: &mut Cursor<'_, CachePage>,
    ) -> Result<(usize, CachePage), VmoCommitError> {
        let index = cursor.index() as usize;
        if let Some(page) = cursor.load() {
            // Mapping callers check initialization through `VmoMapMode`.
            return Ok((index, page.clone()));
        }
        if self.as_backed_vmo().is_some() {
            return Err(VmoCommitError::NeedIo { index });
        }

        // Need to commit. Only Anonymous VMOs can reach here, because VMOs
        // with a backend will return `Err` if the page is not loaded.
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
    ///
    /// If the VMO corresponds to a page cache, callers must hold the page table
    /// lock and insert the committed page atomically to the page table inside
    /// the operation.
    pub(crate) fn try_operate_on_range<F>(
        &self,
        range: &Range<usize>,
        mut operate: F,
        map_mode: VmoMapMode,
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
            let mut commit_fn = || {
                let (idx, page) = self.try_commit_with_cursor(&mut cursor)?;
                map_mode.try_ensure(idx, &page)?;
                Ok((idx, page))
            };
            operate(&mut commit_fn)?;
            cursor.next();
        }
        Ok(())
    }

    /// Returns the current page-aligned size of the VMO in bytes.
    ///
    /// Higher layers may keep a smaller logical EOF alongside this capacity.
    pub(crate) fn size(&self) -> usize {
        self.size.load(Ordering::Acquire)
    }

    /// Returns the status of writable mappings of the VMO.
    pub(crate) fn writable_mapping_status(&self) -> &WritableMappingStatus {
        // Currently, only VMOs used by `MemfdInode` (anonymous) track writable mapping status.
        // VMOs with a backend do not use this field.
        debug_assert!(!self.has_backend());
        &self.writable_mapping_status
    }

    /// Returns reverse mappings of the VMO.
    ///
    /// Holding either this lock or the VMAR lock ensures the stability of the
    /// associated VM mappings. In other words, when adding, removing, or
    /// moving a VM mapping, both this lock and the VMAR lock must be held.
    ///
    /// To avoid deadlocks, acquire this lock after the VMAR lock in cases both
    /// locks need to be acquired.
    pub(crate) fn rmap(&self) -> &Mutex<Rmap> {
        &self.rmap
    }

    /// Decommits anonymous pages in the specified byte range.
    pub(super) fn decommit_anon_pages(
        &self,
        mut locked_rmap: MutexGuard<Rmap>,
        mut locked_pages: LockedXArray<CachePage>,
        range: Range<usize>,
    ) -> Result<()> {
        debug_assert!(!self.has_backend());

        let page_idx_range = get_page_idx_range(&range);
        let mut cursor = locked_pages.cursor_mut(page_idx_range.start as u64);

        // We remove pages before unmapping them in the page table. So a concurrent page fault will
        // synchronize via the page table lock. Either the mapped item will be unmapped below or the
        // page fault handler will commit the page and be blocked on the reverse-mapping lock.
        loop {
            cursor.remove();
            let page_idx = cursor.next_present();
            if page_idx.is_none_or(|idx| idx >= page_idx_range.end as u64) {
                break;
            }
        }

        drop(locked_pages);

        locked_rmap.unmap(page_idx_range.start * PAGE_SIZE..page_idx_range.end * PAGE_SIZE);

        Ok(())
    }

    /// Submits read-side page-cache population for the specified page-index range.
    ///
    /// Missing pages may have backend reads submitted, but this method does not
    /// wait for the prefetched pages to become readable.
    /// If `marker_idx` is specified and that page is newly inserted, it is
    /// marked to trigger the next readahead window.
    pub(super) fn readahead(
        &self,
        page_idx_range: Range<usize>,
        marker_idx: Option<usize>,
    ) -> Result<()> {
        let Some(backed_vmo) = self.as_backed_vmo() else {
            // Anonymous VMOs (e.g., ramfs) have no I/O latency to hide.
            return Ok(());
        };

        backed_vmo.readahead(page_idx_range, marker_idx)
    }

    /// Returns a readahead trigger for a faulting page, consuming its marker if present.
    ///
    /// Returns `None` for cached pages without a consumable marker,
    /// out-of-bounds indices, or VMOs without a backend.
    /// Does not wait for I/O.
    pub(super) fn take_readahead_trigger(&self, page_idx: usize) -> Option<ReadaheadTrigger> {
        self.as_backed_vmo()?;
        let mut locked_pages = self.pages.lock();
        if page_idx >= self.size().div_ceil(PAGE_SIZE) {
            return None;
        }

        match locked_pages.cursor_mut(page_idx as u64).load() {
            None => Some(ReadaheadTrigger::CacheMiss),
            Some(page) if page.take_readahead_marker() => Some(ReadaheadTrigger::Marker),
            // A PTE miss alone does not justify prefetching an already cached page.
            // Reference: <https://elixir.bootlin.com/linux/v6.12.113/source/mm/filemap.c#L3241-L3244>.
            Some(_) => None,
        }
    }

    /// Finds the next absent cache entry, releasing the cache lock between batches.
    pub(super) fn next_missing_page(&self, mut range: Range<usize>) -> Option<usize> {
        while !range.is_empty() {
            let locked_pages = self.pages.lock();
            let nr_pages = self.size().div_ceil(PAGE_SIZE);
            let mut cursor = locked_pages.cursor(range.start as u64);
            for page_idx in range.by_ref().take(Self::PAGE_BATCH_CAPACITY) {
                if page_idx >= nr_pages {
                    return None;
                }
                if cursor.load().is_none() {
                    return Some(page_idx);
                }
                cursor.next();
            }
        }
        None
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

    /// Reads data, using the supplied readahead for VMOs with a backend.
    pub(crate) fn read(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        readahead: Option<&Readahead>,
    ) -> Result<()> {
        let read_len = writer.avail().min(self.size().saturating_sub(offset));
        if read_len == 0 {
            return Ok(());
        }
        let read_end = offset + read_len;

        let range = offset..read_end;
        let mut page_offset = offset % PAGE_SIZE;
        let mut page_batch = Vec::with_capacity(min(
            get_page_idx_range(&range).len(),
            Self::PAGE_BATCH_CAPACITY,
        ));

        if let Some(backed_vmo) = self.as_backed_vmo() {
            backed_vmo.read(&range, &mut page_offset, writer, readahead, &mut page_batch)
        } else {
            self.read_anonymous_pages(&range, &mut page_offset, writer, &mut page_batch)
        }
    }

    /// Writes data from `reader` into the VMO at `offset`.
    pub(crate) fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
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

        if let Some(backed_vmo) = self.as_backed_vmo() {
            backed_vmo.write(&write_range, &mut page_offset, reader, &mut page_batch)
        } else {
            self.write_anonymous_pages(&write_range, &mut page_offset, reader, &mut page_batch)
        }
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

    /// Reads anonymous pages, zero-filling missing pages on demand.
    fn read_anonymous_pages(
        &self,
        range: &Range<usize>,
        page_offset: &mut usize,
        writer: &mut VmWriter,
        page_batch: &mut Vec<(usize, CachePage)>,
    ) -> Result<()> {
        let page_idx_range = get_page_idx_range(range);
        let mut current_idx = page_idx_range.start;

        while current_idx < page_idx_range.end {
            page_batch.clear();
            if let Some(missing_idx) =
                self.collect_cached_pages(current_idx, page_idx_range.end, |_| false, page_batch)
                && page_batch.is_empty()
            {
                let page = self.commit_on_anonymous(missing_idx)?;
                page_batch.push((missing_idx, page));
            }

            for (_, page) in page_batch.iter() {
                page.reader().skip(*page_offset).read_fallible(writer)?;
                *page_offset = 0;
            }

            current_idx = page_batch.last().unwrap().0 + 1;
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
            self.collect_write_pages(
                current_idx,
                page_idx_range.end,
                CommitMode::SkipInit,
                page_batch,
            )?;

            for (_, page) in page_batch.iter() {
                page.writer().skip(*page_offset).write_fallible(reader)?;
                *page_offset = 0;
            }

            // `current_idx < page_idx_range.end` guarantees at least one page is successfully
            // collected here.
            current_idx = page_batch.last().unwrap().0 + 1;
        }

        Ok(())
    }

    /// Appends resident pages to a batch without waiting for backend I/O.
    ///
    /// Returns the first missing index, if any. A page matching `stop_fn` ends
    /// the batch, including that page. Does not allocate or initialize pages.
    fn collect_cached_pages(
        &self,
        start_idx: usize,
        end_idx: usize,
        stop_fn: impl Fn(&CachePage) -> bool,
        pages: &mut Vec<(usize, CachePage)>,
    ) -> Option<usize> {
        let end_idx = end_idx.min(start_idx + pages.capacity());
        let next_idx = start_idx + pages.len();
        let guard = disable_preempt();
        let mut cursor = self.pages.cursor(&guard, next_idx as u64);
        for index in next_idx..end_idx {
            let Some(page) = cursor.load() else {
                return Some(index);
            };
            pages.push((index, page.clone()));
            if stop_fn(&page) {
                return None;
            }
            cursor.next();
        }
        None
    }

    /// Collects a bounded batch of consecutive pages, initializing them when requested.
    ///
    /// This helper keeps the returned pages alive, but it does not pin them in
    /// the `XArray`. Callers must still serialize against `evict_range()` /
    /// `invalidate_range()` with a higher-level invalidation lock before they
    /// take per-page locks, otherwise buffered writes may dirty a page that has
    /// already been detached from the page cache.
    fn collect_write_pages(
        &self,
        start_idx: usize,
        end_idx: usize,
        commit_mode: CommitMode,
        pages: &mut Vec<(usize, CachePage)>,
    ) -> Result<()> {
        let end_idx = end_idx.min(start_idx + pages.capacity());
        pages.clear();
        if end_idx * PAGE_SIZE > self.size() {
            return_errno_with_message!(Errno::EINVAL, "the range to operate exceeds the VMO size");
        }

        while start_idx + pages.len() < end_idx {
            let page_idx = if commit_mode.skips_init() {
                let Some(missing_idx) =
                    self.collect_cached_pages(start_idx, end_idx, |_| false, pages)
                else {
                    break;
                };
                missing_idx
            } else {
                // Partial writes must initialize resident uninitialized pages
                // as well as newly allocated ones.
                start_idx + pages.len()
            };
            // The cursor's critical section has ended before allocating a page.
            let page = self.commit_on_internal(page_idx, commit_mode)?;
            pages.push((page_idx, page));
        }

        Ok(())
    }
}

/// A wrapper around a VMO with a backend that provides specialized operations.
///
/// This structure is created by calling [`Vmo::as_backed_vmo()`] and provides
/// access to backend-specific functionality like reading from storage and
/// managing dirty pages.
pub(crate) struct BackedVmo<'a> {
    vmo: &'a Vmo,
    backend: Arc<dyn PageCacheBackend>,
}

enum PageSelection {
    Flush,
    Evict,
}

impl PageSelection {
    fn should_collect(&self, page: &CachePage) -> bool {
        let state = page.state();
        match self {
            PageSelection::Flush if state.is_dirty() => true,
            PageSelection::Flush if state.is_uninit() => false,
            PageSelection::Flush if let Some(locked_page) = page.try_lock_guard() => {
                locked_page.is_writing_back()
            }
            // We need to wait for all the pages that are being written back to finish before
            // returning from `flush()`. But we don't know whether this page is being writing back
            // because `try_lock_guard()` fails. So let's be conservative and collect this page.
            PageSelection::Flush => true,
            PageSelection::Evict => state.is_up_to_date(),
        }
    }
}

impl<'a> BackedVmo<'a> {
    /// Reads a byte range, advancing readahead at cache misses and markers.
    fn read(
        &self,
        range: &Range<usize>,
        page_offset: &mut usize,
        writer: &mut VmWriter,
        readahead: Option<&Readahead>,
        page_batch: &mut Vec<(usize, CachePage)>,
    ) -> Result<()> {
        let page_idx_range = get_page_idx_range(range);
        let mut current_idx = page_idx_range.start;
        // Include the boundary page, but do not scan past it.
        // Reference: <https://elixir.bootlin.com/linux/v6.12.113/source/mm/filemap.c#L2354-L2359>.
        let stop_fn = |page: &CachePage| {
            page.state().is_uninit() || (readahead.is_some() && page.has_readahead_marker())
        };

        while current_idx < page_idx_range.end {
            let batch_end = current_idx
                .saturating_add(Vmo::PAGE_BATCH_CAPACITY)
                .min(page_idx_range.end);
            page_batch.clear();
            // Consume a cached prefix before handling the next cache miss.
            // Reference: <https://elixir.bootlin.com/linux/v6.12.113/source/mm/filemap.c#L2554-L2572>.
            if let Some(missing_idx) =
                self.collect_cached_pages(current_idx, batch_end, stop_fn, page_batch)
                && page_batch.is_empty()
            {
                if let Some(readahead) = readahead {
                    readahead.on_read_miss(
                        self,
                        missing_idx..page_idx_range.end,
                        page_idx_range.start,
                    );
                }

                if let Some(missing_idx) =
                    self.collect_cached_pages(current_idx, batch_end, stop_fn, page_batch)
                    && page_batch.is_empty()
                {
                    // Readahead did not populate the current page. Leave later
                    // misses for subsequent batches and read only this page.
                    let page = self.commit_on_internal(missing_idx, CommitMode::SkipInit)?;
                    page_batch.push((missing_idx, page));
                }
            }

            // Handle the marked boundary page before waiting for or copying
            // the batch, so prefetching can overlap the requested I/O.
            // Reference: <https://elixir.bootlin.com/linux/v6.12.113/source/mm/filemap.c#L2575-L2586>.
            if let Some(readahead) = readahead {
                let (index, page) = page_batch.last().unwrap();
                if page.has_readahead_marker() {
                    readahead.on_read_marker(self, *index, page, page_idx_range.end - index);
                }
            }

            // Now we start handling the copy for this patch.
            // Previously, we would stop when we hit either a marker page or a missed/uninitialized page, so now we need to
            // `ensure_init` on the last page in the batch. We do this inside the loop rather than outside it,
            // so that asynchronous readahead has a chance to help complete the page initialization.
            let last_index = page_batch.last().unwrap().0;
            for (index, page) in page_batch.iter() {
                if *index == last_index {
                    page.ensure_init(|locked_page| self.backend.read_page(*index, locked_page))?;
                }
                page.reader().skip(*page_offset).read_fallible(writer)?;
                *page_offset = 0;
            }

            // `current_idx < page_idx_range.end` guarantees at least one page is successfully
            // collected here.
            current_idx = page_batch.last().unwrap().0 + 1;
        }

        Ok(())
    }

    /// Writes a byte range, preserving backend data on partial pages.
    fn write(
        &self,
        range: &Range<usize>,
        page_offset: &mut usize,
        reader: &mut VmReader,
        page_batch: &mut Vec<(usize, CachePage)>,
    ) -> Result<()> {
        // VMOs with a backend require dirty tracking and may skip backend reads
        // for page-aligned ranges that will be entirely overwritten.
        if range.len() < PAGE_SIZE {
            return self.write_pages(range, page_offset, reader, CommitMode::Read, page_batch);
        }

        // Split into head (unaligned), middle (aligned), and tail (unaligned).
        let up_align_start = range.start.align_up(PAGE_SIZE);
        let down_align_end = range.end.align_down(PAGE_SIZE);

        if range.start != up_align_start {
            let head = range.start..up_align_start;
            self.write_pages(&head, page_offset, reader, CommitMode::Read, page_batch)?;
        }
        if up_align_start != down_align_end {
            let mid = up_align_start..down_align_end;
            self.write_pages(&mid, page_offset, reader, CommitMode::SkipInit, page_batch)?;
        }
        if down_align_end != range.end {
            let tail = down_align_end..range.end;
            self.write_pages(&tail, page_offset, reader, CommitMode::Read, page_batch)?;
        }

        Ok(())
    }

    /// Writes data to pages with backend dirty tracking.
    ///
    /// Each page is locked before writing to ensure correct state transitions
    /// and marked dirty for later writeback.
    fn write_pages(
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
            self.vmo.collect_write_pages(
                current_idx,
                page_idx_range.end,
                commit_mode,
                page_batch,
            )?;

            // `current_idx < page_idx_range.end` guarantees at least one page is successfully
            // collected here.
            let mut next_idx = page_batch.last().unwrap().0 + 1;

            for (idx, page) in page_batch.drain(..) {
                let locked_page = page.lock();

                let state = locked_page.state();
                if state.is_evicted() {
                    // Retry. We will not see this page again because we've acquired the page lock.
                    next_idx = idx;
                    continue;
                }

                let written_size = match locked_page
                    .writer()
                    .skip(*page_offset)
                    .write_fallible(reader)
                {
                    Ok(written_size) => written_size,
                    Err((err, written_size)) => {
                        // If the page is not initialized, keep it as it is on a partial write.
                        if written_size > 0 && state.is_up_to_date() {
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

    /// Writes back dirty pages in the specified byte range to the backend storage.
    pub(super) fn flush_dirty_pages(&self, range: &Range<usize>) -> Result<()> {
        if range.start >= self.size() {
            return Ok(());
        }

        let mut locked_rmap = self.rmap.lock();

        let page_idx_range = get_page_idx_range(range);
        let pages_to_flush = self.collect_pages_if(&page_idx_range, PageSelection::Flush);
        if pages_to_flush.is_empty() {
            return Ok(());
        }

        // We must lock and mark all pages as up-to-date before freezing them in the page table. A
        // concurrent page fault caused by a write operation will synchronize via the page table
        // lock. Either the mapped item will be freezed below or the page fault handler will see the
        // page as up to date and be blocked on the page lock.
        let mut locked_dirty_pages = Vec::new();
        let mut maybe_writing_back_pages = Vec::new();
        for (idx, page) in pages_to_flush {
            if !page.state().is_dirty() {
                maybe_writing_back_pages.push(page);
                continue;
            }
            let locked_page = page.lock();
            if locked_page.state().is_dirty() {
                locked_page.set_up_to_date();
                locked_dirty_pages.push((idx, locked_page));
            } else if locked_page.is_writing_back() {
                maybe_writing_back_pages.push(locked_page.unlock());
            }
        }

        locked_rmap.freeze(page_idx_range.start * PAGE_SIZE..page_idx_range.end * PAGE_SIZE);

        // As long as the pages are locked and their state is `UpToDate`, no one can write to them.
        // Therefore, we can release the reverse-mapping lock now.
        drop(locked_rmap);

        let mut io_batch = IoBatch::with_capacity(locked_dirty_pages.len());
        for locked_page_run in locked_dirty_pages
            .chunk_by_mut(|(page_idx, _), (next_page_idx, _)| page_idx + 1 == *next_page_idx)
        {
            // Single page in one run.
            if locked_page_run.len() == 1 {
                let (idx, locked_page) = &mut locked_page_run[0];
                self.backend
                    .write_page_async(*idx, locked_page.detach(), &mut io_batch)?;
                continue;
            }

            // Multiple pages in one run.
            let start_idx = locked_page_run[0].0;
            let mut locked_pages = locked_page_run
                .iter_mut()
                .map(|(_, locked_page)| locked_page.detach());
            let pages = PageRun::new(start_idx, &mut locked_pages);
            if let Err(err) = self.backend.write_pages_async(pages, &mut io_batch) {
                for locked_page in locked_pages {
                    locked_page.set_dirty();
                }
                let _ = io_batch.wait_all();
                return Err(err);
            }
        }
        io_batch.wait_all()?;

        for page in maybe_writing_back_pages {
            let locked_page = page.lock();
            locked_page.wait_until_finish_writing_back();
        }

        Ok(())
    }

    /// Removes up-to-date (clean) pages in the specified byte range from the page cache.
    ///
    /// Only pages in the `UpToDate` state are removed. Dirty and uninitialized
    /// pages are left in place.
    pub(super) fn evict_up_to_date_pages(&self, range: &Range<usize>) -> Result<()> {
        if range.start >= self.size() {
            return Ok(());
        }

        let mut locked_rmap = self.rmap.lock();

        let page_idx_range = get_page_idx_range(range);
        let up_to_date_pages = self.collect_pages_if(&page_idx_range, PageSelection::Evict);
        if up_to_date_pages.is_empty() {
            return Ok(());
        }

        // We need to lock all pages and ensure they are up to date under the page lock. In
        // addition, we mark pages to be removed as `Evicted` before unmapping them in the page
        // table. A concurrent page fault will synchronize via the page table lock. Either the
        // mapped item will be unmapped below or the page fault handler will see the page as removed
        // and be blocked on the page lock.
        let locked_up_to_date_pages = up_to_date_pages
            .into_iter()
            .filter_map(|(idx, page)| {
                let locked_page = page.lock();
                if !locked_page.state().is_up_to_date() {
                    return None;
                }
                locked_page.set_evicted();
                Some((idx, locked_page))
            })
            .collect::<Vec<_>>();
        if locked_up_to_date_pages.is_empty() {
            return Ok(());
        }

        locked_rmap.unmap(page_idx_range.start * PAGE_SIZE..page_idx_range.end * PAGE_SIZE);

        // As long as the pages are locked and their state is `Evicted`, no one can read from them.
        // Therefore, we can release the reverse-mapping lock now.
        drop(locked_rmap);

        for (idx, page) in locked_up_to_date_pages {
            page.wait_until_finish_writing_back();
            let mut locked_pages = self.pages.lock();
            let mut cursor_mut = locked_pages.cursor_mut(idx as u64);
            let removed_page = cursor_mut.remove();
            // The page won't be evicted because it has already been marked as `Evicted`. It won't
            // be decommitted either, as the caller should hold a file system lock that prevents
            // concurrent resizing.
            //
            // TODO: If we allowing pages to be evicted without holding the file system lock, the
            // old page may be decommitted and replaced by another page. In this case, we must
            // reload and recheck the page before removing it.
            debug_assert_eq!(removed_page.unwrap().paddr(), page.paddr());
        }

        Ok(())
    }

    /// Decommits pages with a backend in the specified byte range.
    ///
    /// Dirty pages in the range are discarded without being written back. If a
    /// page is already under writeback, decommit waits for that writeback to
    /// finish before removing the page from the `XArray`.
    pub(super) fn decommit_pages(
        &self,
        mut locked_rmap: MutexGuard<Rmap>,
        locked_pages: LockedXArray<CachePage>,
        range: &Range<usize>,
    ) -> Result<()> {
        // Do not check `self.size()` here. It contains the new size, however, we want to decommit
        // pages outside of the new range. See `PageCache::resize` for a concrete example.

        // We remove pages before unmapping them in the page table. So a concurrent page fault will
        // synchronize via the page table lock. Either the mapped item will be unmapped below or the
        // page fault handler will see the new size after acquiring the lock in `self.pages`.
        let page_idx_range = get_page_idx_range(range);
        let removed_pages = self.remove_pages_in(locked_pages, &page_idx_range);
        if removed_pages.is_empty() {
            return Ok(());
        }

        locked_rmap.unmap(page_idx_range.start * PAGE_SIZE..page_idx_range.end * PAGE_SIZE);

        drop(locked_rmap);

        for (_, page) in removed_pages {
            let locked_page = page.lock();
            locked_page.wait_until_finish_writing_back();
        }

        Ok(())
    }

    /// Commits a page at the given index for a VMO with a backend.
    fn commit_on_internal(&self, page_idx: usize, commit_mode: CommitMode) -> Result<CachePage> {
        let mut locked_pages = self.pages.lock();
        if page_idx >= self.size().div_ceil(PAGE_SIZE) {
            return_errno_with_message!(Errno::EINVAL, "the offset is outside the VMO");
        }

        let mut cursor = locked_pages.cursor_mut(page_idx as u64);

        let page = if let Some(page) = cursor.load() {
            page.clone()
        } else {
            let page = CachePage::alloc_uninit()?;
            cursor.store(page.clone());
            page
        };
        drop(locked_pages);

        if !commit_mode.skips_init() {
            page.ensure_init(|locked_page| self.backend.read_page(page_idx, locked_page))?;
        }
        Ok(page)
    }

    /// Collects pages in the specified page index range.
    fn collect_pages_if(
        &self,
        page_idx_range: &Range<usize>,
        cond: PageSelection,
    ) -> Vec<(usize, CachePage)> {
        if page_idx_range.is_empty() {
            return Vec::new();
        }

        let mut collected_pages = Vec::new();

        let preempt_guard = disable_preempt();
        let mut cursor = self
            .pages
            .cursor(&preempt_guard, page_idx_range.start as u64);
        let mut index = page_idx_range.start;

        loop {
            if let Some(page) = cursor.load()
                && cond.should_collect(&page)
            {
                collected_pages.push((index, page.clone()));
            }

            let Some(next_index) = cursor.next_present() else {
                break;
            };
            index = next_index as usize;
            if index >= page_idx_range.end {
                break;
            }
        }

        collected_pages
    }

    /// Removes pages in the specified page index range.
    fn remove_pages_in(
        &self,
        mut locked_pages: LockedXArray<CachePage>,
        page_idx_range: &Range<usize>,
    ) -> Vec<(usize, CachePage)> {
        if page_idx_range.is_empty() {
            return Vec::new();
        }

        let mut removed_pages = Vec::new();

        let mut cursor_mut = locked_pages.cursor_mut(page_idx_range.start as u64);
        let mut index = page_idx_range.start;

        loop {
            if let Some(page) = cursor_mut.remove() {
                removed_pages.push((index, page.clone()));
            }

            let Some(next_index) = cursor_mut.next_present() else {
                break;
            };
            index = next_index as usize;
            if index >= page_idx_range.end {
                break;
            }
        }

        removed_pages
    }

    /// Submits read I/O for pages in the specified page-index range.
    fn readahead(&self, page_idx_range: Range<usize>, marker_idx: Option<usize>) -> Result<()> {
        let page_count = self.size().div_ceil(PAGE_SIZE);
        let readahead_end_idx = page_idx_range.end.min(page_count);
        if page_idx_range.start >= readahead_end_idx {
            return Ok(());
        }

        let mut io_batch = None;
        let mut page_idx = page_idx_range.start;
        while let Some(start_idx) = self.next_missing_page(page_idx..readahead_end_idx) {
            let (next_idx, pages) =
                self.collect_readahead_run(start_idx, readahead_end_idx, marker_idx)?;
            if next_idx == start_idx {
                break;
            }
            if !pages.is_empty() {
                let mut locked_pages = pages.into_iter();
                let pages = PageRun::new(start_idx, &mut locked_pages);
                let io_batch = io_batch
                    .get_or_insert_with(|| IoBatch::with_capacity(Vmo::PAGE_BATCH_CAPACITY));
                self.backend.read_pages_async(pages, io_batch)?;
            }
            page_idx = next_idx;
        }

        Ok(())
    }

    /// Collects a bounded run of consecutive missing pages for readahead.
    fn collect_readahead_run(
        &self,
        start_idx: usize,
        end_idx: usize,
        marker_idx: Option<usize>,
    ) -> Result<(usize, Vec<LockedCachePage>)> {
        let mut pages = Vec::with_capacity(Vmo::PAGE_BATCH_CAPACITY);
        let mut locked_pages = self.vmo.pages.lock();
        let end_idx = end_idx.min(self.size().div_ceil(PAGE_SIZE));
        let mut page_idx = start_idx;
        while page_idx < end_idx && pages.len() < Vmo::PAGE_BATCH_CAPACITY {
            let mut cursor = locked_pages.cursor_mut(page_idx as u64);
            if cursor.load().is_some() {
                page_idx += 1;
                break;
            }

            let page = CachePage::alloc_uninit()?;
            let Some(locked_page) = page.try_lock() else {
                break;
            };
            if marker_idx == Some(page_idx) {
                locked_page.set_readahead_marker();
            }
            cursor.store(locked_page.clone());
            pages.push(locked_page);
            page_idx += 1;
        }

        Ok((page_idx, pages))
    }
}

impl Deref for BackedVmo<'_> {
    type Target = Vmo;

    fn deref(&self) -> &Self::Target {
        self.vmo
    }
}

/// Gets the page index range that contains the offset range of VMO.
pub(crate) fn get_page_idx_range(vmo_offset_range: &Range<usize>) -> Range<usize> {
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
pub(crate) struct WritableMappingStatus(AtomicIsize);

impl WritableMappingStatus {
    /// Builds a new writable mapping.
    ///
    /// Fails with `EPERM` if writable mappings have been denied.
    pub(crate) fn map(&self) -> Result<()> {
        // Increase unless negative
        self.0
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                (v >= 0).then(|| v + 1)
            })
            .map_err(|_| Error::with_message(Errno::EPERM, "writable mappings have been denied"))?;
        Ok(())
    }

    /// Denies any future writable mapping.
    ///
    /// Fails with `EBUSY` if there are still active writable mappings.
    pub(crate) fn deny(&self) -> Result<()> {
        // Decrease unless positive
        self.0
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
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
    pub(crate) fn increment(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrements the writable mapping counter.
    ///
    /// Typically used when unmapping a region, exiting a process, or merging mappings.
    pub(crate) fn decrement(&self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
