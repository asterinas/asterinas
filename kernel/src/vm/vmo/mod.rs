// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

//! Virtual Memory Objects (VMOs).

use core::{
    ops::{Deref, Range},
    sync::atomic::{AtomicIsize, AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use ostd::{
    mm::{VmIo, VmIoFill, VmReader, VmWriter, io::util::HasVmReaderWriter},
    task::disable_preempt,
};
use xarray::{Cursor, LockedXArray, XArray};

use crate::{
    fs::utils::{CachePage, CachePageExt, PageCacheBackend},
    prelude::*,
};

mod options;
mod page_cache;

pub use options::VmoOptions;

/// Virtual Memory Objects (VMOs) represent contiguous ranges of virtual memory pages.
///
/// VMOs serve as the fundamental building blocks for memory management in Asterinas,
/// providing a unified interface for both anonymous (RAM-backed) and disk-backed memory.
///
/// # Types of VMOs
///
/// There are two primary types of VMOs, distinguished by their backing storage:
///
/// 1. **Anonymous VMO**: Backed purely by RAM with no persistent storage. Pages are
///    initially zero-filled and exist only in memory. These are typically used for
///    heap allocations, anonymous mappings, and stack memory.
///
/// 2. **Disk-backed VMO**: Associated with a disk-backed file through a [`PageCacheBackend`].
///    Pages are lazily loaded from the file on first access and can be written back
///    to storage. These VMOs integrate with the page cache for efficient file I/O
///    and memory-mapped files.
///
/// # Features
///
///  * **I/O interface.** A VMO provides read and write methods to access the
///    memory pages that it contain.
///  * **On-demand paging.** The memory pages of a VMO (except for _contiguous_
///    VMOs) are allocated lazily when the page is first accessed.
///  * **Device driver support.** If specified upon creation, VMOs will be
///    backed by physically contiguous memory pages starting at a target address.
///  * **File system support.** By default, a VMO's memory pages are initially
///    all zeros. But if a VMO is attached to a backend ([`PageCacheBackend`]) upon creation,
///    then its memory pages will be populated by the backend.
///    With this backend mechanism, file systems can easily implement page caches
///    with VMOs by attaching the VMOs to backends backed by inodes.
///
/// # Examples
///
/// For creating root VMOs, see [`VmoOptions`].
///
/// # Implementation
///
/// `Vmo` provides high-level APIs for address space management by wrapping
/// around its low-level counterpart [`CachePage`]. Compared with [`CachePage`],
/// `Vmo` is easier to use (by offering more powerful APIs) and
/// harder to misuse (thanks to its nature of being capability).
///
/// # Concurrency Guarantees
///
/// A `Vmo` guarantees the correctness of each [`CachePage`]'s [`PageState`]
/// transitions (e.g., `Uninit` → `UpToDate` → `Dirty`). These transitions are
/// performed atomically under the page lock, ensuring that concurrent readers
/// and writers always observe a consistent page state. For anonymous VMOs,
/// pages are always stored in the `UpToDate` state.
///
/// However, a `Vmo` does **not** guarantee atomicity of the page *contents*
/// with respect to concurrent reads and writes. In particular, when a page is
/// mapped into user-space address space, the kernel cannot prevent data races
/// between concurrent user-space memory accesses and kernel-side I/O operations
/// (e.g., `read`/`write` system calls or page fault handling). Callers that
/// require stronger consistency guarantees must provide their own
/// synchronization (e.g., file locks or application-level mutexes).
///
/// [`PageState`]: crate::fs::utils::PageState
pub struct Vmo {
    /// The backend that provides disk I/O operations, if any.
    //
    // TODO: Using `Weak` here is to avoid circular references in exfat file systems.
    // We should avoid the circular reference by design, and then we can change this to `Arc`.
    backend: Option<Weak<dyn PageCacheBackend>>,
    /// Flags
    flags: VmoFlags,
    /// The virtual pages where the VMO resides.
    pages: XArray<CachePage>,
    /// The size of the VMO.
    ///
    /// Note: This size may not necessarily match the size of the `pages`, but it is
    /// required here that modifications to the size can only be made after locking
    /// the [`XArray`] in the `pages` field. Therefore, the size read after locking the
    /// `pages` will be the latest size.
    size: AtomicUsize,
    /// The status of writable mappings of the VMO.
    //
    // TODO: This field is used only by VMOs belonging to memfd (i.e., `MemfdInode`). But VMOs do
    // not have the knowledge to determine if they belong to memfd. We may want to enhance
    // `VmoOptions` to make VMOs aware of whether its writable mappings should be tracked.
    writable_mapping_status: WritableMappingStatus,
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
        self.commit_on_internal(page_idx, false)
    }

    fn commit_on_internal(&self, page_idx: usize, will_overwrite: bool) -> Result<CachePage> {
        let mut locked_pages = self.pages.lock();
        if page_idx * PAGE_SIZE > self.size() {
            return_errno_with_message!(Errno::EINVAL, "the offset is outside the VMO");
        }

        if let Some(disk_backed) = self.as_disk_backed() {
            disk_backed.commit_on(locked_pages, page_idx, will_overwrite)
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
    /// If the commit operation needs to perform I/O, it will return a [`VmoCommitError::NeedIo`].
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
        self.try_commit_with_cursor(&mut cursor, false)
            .map(|(_, page)| page)
    }

    fn try_commit_with_cursor(
        &self,
        cursor: &mut Cursor<'_, CachePage>,
        will_overwrite: bool,
    ) -> core::result::Result<(usize, CachePage), VmoCommitError> {
        if let Some(disk_backed) = self.as_disk_backed() {
            if let Some((index, page)) =
                disk_backed.try_commit_with_cursor(cursor, will_overwrite)?
            {
                return Ok((index, page));
            }
        } else if let Some(page) = cursor.load() {
            let index = cursor.index() as usize;
            return Ok((index, page.clone()));
        }

        // Need to commit. Only Anonymous VMOs can reach here, because disk-backed VMOs will return
        // `Err` if the page is not loaded.
        let index = cursor.index() as usize;
        Ok((index, self.commit_on_internal(index, will_overwrite)?))
    }

    /// Traverses the indices within a specified range of a VMO sequentially.
    ///
    /// For each index position, you have the option to commit the page as well as
    /// perform other operations.
    ///
    /// Once a commit operation needs to perform I/O, it will return a [`VmoCommitError::NeedIo`].
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
        self.try_operate_on_range_internal(range, operate, false)
    }

    fn try_operate_on_range_internal<F>(
        &self,
        range: &Range<usize>,
        mut operate: F,
        will_overwrite: bool,
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
            let mut commit_fn = || self.try_commit_with_cursor(&mut cursor, will_overwrite);
            operate(&mut commit_fn)?;
            cursor.next();
        }
        Ok(())
    }

    /// Returns the size of current VMO.
    pub fn size(&self) -> usize {
        self.size.load(Ordering::Acquire)
    }

    /// Returns the flags of current VMO.
    pub fn flags(&self) -> VmoFlags {
        self.flags
    }

    /// Returns the status of writable mappings of the VMO.
    pub fn writable_mapping_status(&self) -> &WritableMappingStatus {
        // Currently, only VMOs used by `MemfdInode` (anonymous) track writable mapping status.
        // Disk-backed VMOs do not use this field.
        debug_assert!(!self.is_disk_backed());
        &self.writable_mapping_status
    }

    fn decommit_pages(
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
    fn as_disk_backed(&self) -> Option<DiskBackedVmo<'_>> {
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
    fn handle_commit_error(&self, err: VmoCommitError, will_overwrite: bool) -> Result<usize> {
        match err {
            VmoCommitError::Err(e) => Err(e),
            VmoCommitError::NeedIo(index) => {
                self.commit_on_internal(index, will_overwrite)?;
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
    fn collect_pages(
        &self,
        start_idx: usize,
        end_idx: usize,
        will_overwrite: bool,
    ) -> Result<Vec<(usize, CachePage)>> {
        /// Maximum number of pages collected per batch from the `XArray`.
        const PAGE_BATCH_CAPACITY: usize = 32;

        let mut pages = Vec::new();

        let end_idx = end_idx.min(start_idx + PAGE_BATCH_CAPACITY);
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
            match self.try_operate_on_range_internal(&current_range, &mut operate, will_overwrite) {
                Ok(()) => break 'retry,
                Err(err) => {
                    let idx = self.handle_commit_error(err, will_overwrite)?;
                    current_range.start = idx * PAGE_SIZE;
                }
            }
        }

        Ok(pages)
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

        while current_idx < page_idx_range.end {
            let pages = self.collect_pages(current_idx, page_idx_range.end, false)?;

            for (_, page) in &pages {
                page.reader()
                    .skip(page_offset)
                    .read_fallible(writer)
                    .map_err(|e| Error::from(e.0))?;
                page_offset = 0;
            }

            // `current_idx < page_idx_range.end` guarantees at least one page is successfully collected here.
            current_idx = pages.last().unwrap().0 + 1;
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

        if !self.is_disk_backed() {
            return self.write_anonymous_pages(&write_range, &mut page_offset, reader);
        }

        // Disk-backed VMOs require dirty tracking and may skip backend reads
        // for page-aligned ranges that will be entirely overwritten.
        if write_range.len() < PAGE_SIZE {
            return self.write_disk_backed_pages(&write_range, &mut page_offset, reader, false);
        }

        // Split into head (unaligned), middle (aligned), and tail (unaligned).
        let up_align_start = write_range.start.align_up(PAGE_SIZE);
        let down_align_end = write_range.end.align_down(PAGE_SIZE);

        if write_range.start != up_align_start {
            let head = write_range.start..up_align_start;
            self.write_disk_backed_pages(&head, &mut page_offset, reader, false)?;
        }
        if up_align_start != down_align_end {
            let mid = up_align_start..down_align_end;
            self.write_disk_backed_pages(&mid, &mut page_offset, reader, true)?;
        }
        if down_align_end != write_range.end {
            let tail = down_align_end..write_range.end;
            self.write_disk_backed_pages(&tail, &mut page_offset, reader, false)?;
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
    ) -> Result<()> {
        let page_idx_range = get_page_idx_range(range);
        let mut current_idx = page_idx_range.start;

        while current_idx < page_idx_range.end {
            let pages = self.collect_pages(current_idx, page_idx_range.end, false)?;

            for (_, page) in &pages {
                page.writer()
                    .skip(*page_offset)
                    .write_fallible(reader)
                    .map_err(|e| Error::from(e.0))?;
                *page_offset = 0;
            }

            // `current_idx < page_idx_range.end` guarantees at least one page is successfully collected here.
            current_idx = pages.last().unwrap().0 + 1;
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
        will_overwrite: bool,
    ) -> Result<()> {
        let page_idx_range = get_page_idx_range(range);
        let mut current_idx = page_idx_range.start;

        while current_idx < page_idx_range.end {
            let pages = self.collect_pages(current_idx, page_idx_range.end, will_overwrite)?;

            // `current_idx < page_idx_range.end` guarantees at least one page is successfully collected here.
            let next_idx = pages.last().unwrap().0 + 1;

            for (_, page) in pages {
                let locked_page = page.lock();
                locked_page.set_dirty();
                locked_page
                    .writer()
                    .skip(*page_offset)
                    .write_fallible(reader)
                    .map_err(|e| Error::from(e.0))?;
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
        will_overwrite: bool,
    ) -> Result<CachePage> {
        let mut cursor = locked_pages.cursor_mut(page_idx as u64);
        if let Some(page) = cursor.load() {
            let page = page.clone();
            if self.backend.npages() > page_idx {
                drop(locked_pages);
                if !will_overwrite {
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

            cursor.store(locked_page.clone());

            drop(locked_pages);

            if will_overwrite {
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
        will_overwrite: bool,
    ) -> core::result::Result<Option<(usize, CachePage)>, VmoCommitError> {
        let page_idx = cursor.index() as usize;

        let Some(page) = cursor.load() else {
            return Err(VmoCommitError::NeedIo(page_idx));
        };

        // If page is within file bounds, check if it's initialized
        if !will_overwrite && self.backend.npages() > page_idx && page.is_uninit() {
            return Err(VmoCommitError::WaitUntilInit(page_idx, page.clone()));
        }

        Ok(Some((page_idx, page.clone())))
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

    /// Removes up-to-date (clean) pages in the specified byte range from the page cache.
    ///
    /// Only pages in the `UpToDate` state are removed. Dirty and uninitialized
    /// pages are left in place.
    //
    // TODO: Returns `Err` if any up-to-date page has been mapped.
    fn evict_up_to_date_pages(&self, range: &Range<usize>) -> Result<()> {
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

    /// Writes back a collection of dirty pages to the backend storage.
    fn write_back_pages(&self, dirty_pages: Vec<(usize, CachePage)>) -> Result<()> {
        for (page_idx, page) in dirty_pages {
            let locked_page = page.lock();
            if locked_page.is_dirty() {
                self.backend.write_page(page_idx, locked_page)?;
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
    pub(super) fn map(&self) -> Result<()> {
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
    pub(super) fn increment(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrements the writable mapping counter.
    ///
    /// Typically used when unmapping a region, exiting a process, or merging mappings.
    pub(super) fn decrement(&self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
