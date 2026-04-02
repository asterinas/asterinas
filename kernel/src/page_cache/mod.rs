// SPDX-License-Identifier: MPL-2.0

//! Page-cache infrastructure shared by filesystems, memory mappings, and
//! writeback.
//!
//! This module is the filesystem-facing entry point to the page-cache
//! subsystem. A filesystem typically keeps one [`PageCache`] per file (or per
//! inode data stream) and uses it for buffered I/O, truncate/extend, and cache
//! invalidation.
//!
//! # Overview
//!
//! The subsystem is intentionally split into three layers with different
//! responsibilities:
//!
//! - [`PageCache`] is the per-file façade used by filesystems. It exposes
//!   buffered I/O, resize, flush, and invalidation operations in filesystem
//!   terms.
//! - [`Vmo`] is the lower-level memory object underneath a page cache. It owns
//!   the page array, commits pages on demand, and is the abstraction shared
//!   with page-fault and mapping code.
//! - [`PageCacheBackend`] together with [`CachePage`] /
//!   [`cache_page::PageState`] define how individual pages are populated from,
//!   and written back to, persistent storage.
//!
//! For a disk-backed file, the steady-state flow is
//! `filesystem metadata -> PageCache -> Vmo -> PageCacheBackend -> block
//! device`. Anonymous page caches use the same `PageCache` / `Vmo` layers
//! without a backend.
//!
//! # Responsibility Boundary
//!
//! `PageCache` manages page-aligned cache capacity and cached contents. The
//! filesystem still owns logical file size, extent or block mapping metadata,
//! and the higher-level locking that keeps those decisions stable while the
//! page cache is accessed.
//!
//! Stay at [`PageCache`] when the caller is operating in filesystem terms. Drop
//! to [`Vmo`] only when code needs the lower-level memory-object interface
//! directly, such as `mmap` setup or page-fault handling.
//!
//! # Synchronization Model
//!
//! The page-cache subsystem serializes per-page state transitions, including
//! the disk-backed states in [`cache_page::PageState`] and the auxiliary
//! writeback tracking bit in [`CachePageMeta`]. It does not provide whole-file
//! serialization.
//! Callers still need inode- or file-level critical sections for:
//!
//! - buffered I/O that depends on stable file size or extent metadata;
//! - [`PageCache::flush_range`] when `fsync`-like write ordering matters;
//! - [`PageCache::evict_range`] / [`PageCache::invalidate_range`] that must
//!   exclude buffered writes or page faults over the same range; and
//! - resize, truncate, or extend operations that update both inode metadata and
//!   page-cache capacity.
//!
//! See [`PageCache`] for the filesystem-facing contract and [`Vmo`] for the
//! lower-level memory-object contract.

use core::{
    ops::{Deref, Range},
    sync::atomic::Ordering,
};

use align_ext::AlignExt;
use aster_block::bio::{BioCompleteFn, BioDirection, BioSegment, BioStatus, BioWaiter};
use ostd::mm::{Segment, VmIo, VmIoFill, io_util::HasVmReaderWriter};

use crate::prelude::*;

pub mod cache_page;
#[cfg(ktest)]
mod tests;
pub mod vmo;

pub use cache_page::{CachePage, CachePageExt, CachePageMeta};
pub use vmo::{Vmo, VmoCommitError, VmoFlags, VmoOptions, WritableMappingStatus};

/// The page cache for a file-like object.
///
/// This is the abstraction a filesystem usually stores in an inode: it handles
/// buffered reads and writes, writeback, invalidation, and page-cache resizing,
/// while delegating per-page population and writeback to the underlying [`Vmo`].
///
/// A `PageCache` owns cached page contents and a page-aligned capacity. It does
/// not own the filesystem's logical file size, extent metadata, or invalidation
/// policy. Callers must keep those pieces synchronized with page-cache
/// operations, especially around [`PageCache::resize`],
/// [`PageCache::flush_range`], and [`PageCache::invalidate_range`].
///
/// Disk-backed filesystems create it with [`PageCache::new_disk_backed`] and a
/// [`PageCacheBackend`]. Purely in-memory filesystems can use
/// [`PageCache::new_anon`] to get the same buffered-I/O interface without a
/// persistent backend.
///
/// Reach for [`PageCache::as_vmo`] only when a lower-level consumer such as
/// memory mapping or page-fault code must operate on the underlying [`Vmo`]
/// directly.
#[derive(Clone, Debug)]
pub struct PageCache(Arc<Vmo>);

impl PageCache {
    /// Returns the wrapped [`Vmo`].
    pub fn as_vmo(&self) -> Arc<Vmo> {
        self.0.clone()
    }

    /// Creates a disk-backed page cache with the specified initial capacity in
    /// bytes.
    pub fn new_disk_backed(size: usize, backend: Weak<dyn PageCacheBackend>) -> Result<Self> {
        Ok(Self::from(
            VmoOptions::new_page_cache(size, backend).alloc()?,
        ))
    }

    /// Creates an anonymous page cache with the specified initial capacity in
    /// bytes.
    pub fn new_anon(size: usize) -> Result<Self> {
        Ok(Self::from(VmoOptions::new_anon(size).alloc()?))
    }

    /// Returns the current page-cache capacity in bytes.
    ///
    /// This size is page-aligned and may exceed the file's logical size. The
    /// filesystem remains responsible for tracking logical EOF separately.
    pub fn size(&self) -> usize {
        self.0.size()
    }

    /// Returns the writable mapping status of the underlying VMO.
    pub fn writable_mapping_status(&self) -> &WritableMappingStatus {
        &self.0.writable_mapping_status
    }

    /// Resizes the page-cache capacity to cover a new logical file size.
    ///
    /// `new_size` is the post-resize logical size requested by the filesystem.
    /// The underlying cache capacity is rounded up to page boundaries. If the
    /// new size is smaller than the current size, pages that fall entirely
    /// within the truncated range will be decommitted (freed). For the page
    /// that is only partially truncated (i.e., the page containing the new
    /// boundary), the truncated portion will be filled with zeros instead.
    ///
    /// The `logical_file_size` must be the logical file length before this resize.
    /// It is used to determine the boundary of previously valid data so that
    /// only the discarded logical range within a partially truncated tail page
    /// is zero-filled.
    ///
    /// # Size Synchronization
    ///
    /// `PageCache::resize` only updates the page-aligned [`Vmo`] capacity. The
    /// filesystem must keep that capacity synchronized with its own logical
    /// file size under the same inode- or file-level lock that excludes
    /// conflicting buffered I/O, page faults, and invalidation.
    ///
    /// The required ordering is:
    ///
    /// - When extending a file, update the logical file size before
    ///   increasing [`Vmo::size`] so subsequent reads can observe the new
    ///   range.
    /// - When truncating a file, shrink [`Vmo::size`] before decreasing the
    ///   logical file size so reads beyond the new EOF cannot observe stale
    ///   cached pages.
    ///
    /// Accordingly, `logical_file_size` must be the pre-resize logical file
    /// size captured inside that resize critical section.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ostd::mm::PAGE_SIZE;
    ///
    /// use crate::page_cache::PageCache;
    ///
    /// let page_cache = PageCache::new_anon(0).unwrap();
    ///
    /// // Extend: publish the new logical file size first, then grow the page
    /// // cache with the previous logical size.
    /// let mut logical_file_size = PAGE_SIZE + 123;
    /// page_cache.resize(logical_file_size, 0).unwrap();
    /// assert_eq!(page_cache.size(), 2 * PAGE_SIZE);
    ///
    /// // Truncate: shrink the page cache first, while passing the old logical
    /// // file size captured in the same critical section.
    /// let old_logical_file_size = logical_file_size;
    /// logical_file_size = 512;
    /// page_cache
    ///     .resize(logical_file_size, old_logical_file_size)
    ///     .unwrap();
    /// assert_eq!(page_cache.size(), PAGE_SIZE);
    /// ```
    ///
    /// TODO: Integrate a reverse-mapping lock or equivalent synchronization so
    /// shrink/truncate can coordinate with mapped pages and concurrent page
    /// faults.
    pub fn resize(&self, new_size: usize, logical_file_size: usize) -> Result<()> {
        let vmo = &self.0;
        assert!(vmo.flags.contains(VmoFlags::RESIZABLE));

        if new_size < logical_file_size && !new_size.is_multiple_of(PAGE_SIZE) {
            let fill_zero_end = logical_file_size.min(new_size.align_up(PAGE_SIZE));
            self.fill_zeros(new_size..fill_zero_end)?;
        }

        let new_size = new_size.align_up(PAGE_SIZE);
        let locked_pages = vmo.pages.lock();
        let old_size = vmo.size();
        if new_size == old_size {
            return Ok(());
        }

        vmo.size.store(new_size, Ordering::Release);

        if new_size < old_size {
            vmo.decommit_pages(locked_pages, new_size..old_size)?;
        }

        Ok(())
    }

    /// Flushes dirty pages in the specified range to the backend storage.
    ///
    /// This walks the current cache contents, submits writeback for pages that
    /// are dirty when this pass reaches them, and waits for the submitted I/O
    /// to complete.
    ///
    /// This operation does not hold the `XArray` lock for the whole range while
    /// flushing. Concurrent writers may therefore dirty new pages, or re-dirty
    /// pages that this pass has already processed, before `flush_range()`
    /// returns. A successful return only means that the pages selected by this
    /// writeback pass have been written back; it does not guarantee that no
    /// dirty pages remain in the range afterwards.
    ///
    /// Filesystems that need `fsync`-like guarantees must still exclude
    /// concurrent writers or repeat the operation until their own ordering
    /// requirements are met.
    ///
    /// If the given range exceeds the current size of the page cache, only the pages within
    /// the valid range will be flushed.
    pub fn flush_range(&self, range: Range<usize>) -> Result<()> {
        let Some(vmo) = self.0.as_disk_backed() else {
            return Ok(());
        };

        vmo.flush_dirty_pages(&range)
    }

    /// Evicts clean pages within the specified range from the page cache.
    ///
    /// Only pages in the `UpToDate` state are removed. Dirty and uninitialized
    /// pages are left in place. This is useful for invalidating cached data that
    /// is no longer needed and can be re-read from the backend if necessary
    /// (e.g., before direct I/O operations).
    ///
    /// TODO: Integrate a reverse-mapping lock or equivalent synchronization so
    /// eviction can coordinate with mapped pages and concurrent page faults
    /// before removing cached pages from the page cache.
    pub fn evict_range(&self, range: Range<usize>) -> Result<()> {
        let Some(vmo) = self.0.as_disk_backed() else {
            return Ok(());
        };

        vmo.evict_up_to_date_pages(&range)
    }

    /// Flushes dirty pages and then evicts clean pages in the specified range.
    ///
    /// This is the standard preparation step before issuing direct I/O that must
    /// bypass the page cache. It uses the same locking requirements as
    /// [`PageCache::flush_range`] and [`PageCache::evict_range`]: callers
    /// must exclude concurrent buffered writes and page faults for the same file
    /// range with a higher-level lock.
    pub fn invalidate_range(&self, range: Range<usize>) -> Result<()> {
        self.flush_range(range.clone())?;
        self.evict_range(range)
    }

    /// Fills the specified range of the page cache with zeros.
    pub fn fill_zeros(&self, range: Range<usize>) -> Result<()> {
        VmIoFill::fill_zeros(self, range.start, range.end - range.start)?;
        Ok(())
    }
}

impl From<Arc<Vmo>> for PageCache {
    fn from(vmo: Arc<Vmo>) -> Self {
        Self(vmo)
    }
}

impl VmIo for PageCache {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> ostd::Result<()> {
        self.0.read(offset, writer)?;
        Ok(())
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> ostd::Result<()> {
        self.0.write(offset, reader)?;
        Ok(())
    }
}

impl VmIoFill for PageCache {
    fn fill_zeros(
        &self,
        offset: usize,
        len: usize,
    ) -> core::result::Result<(), (ostd::Error, usize)> {
        VmIoFill::fill_zeros(&*self.0, offset, len)
    }
}

/// This trait represents the backend for the page cache.
///
/// Implementors only need to provide the low-level I/O submission hooks
/// (`submit_read_io` and `submit_write_io`). The trait provides default
/// implementations for `read_page_async` and `write_page_async`, while the VMO
/// layer owns the page-cache state transitions around writeback.
pub trait PageCacheBackend: Sync + Send {
    /// Submits the backend read I/O for the page at the given index.
    ///
    /// The `bio_segment` is the target memory to read into, and `complete_fn` should
    /// be passed to the block device's async read API. The implementor should
    /// not manage page-cache state here; that is handled by the default
    /// `read_page_async`.
    fn submit_read_io(
        &self,
        idx: usize,
        bio_segment: BioSegment,
        complete_fn: Option<BioCompleteFn>,
    ) -> Result<BioWaiter>;

    /// Submits the backend write I/O for the page at the given index.
    ///
    /// The `bio_segment` is the source memory to write from, and `complete_fn` should
    /// be passed to the block device's async write API.
    fn submit_write_io(
        &self,
        idx: usize,
        bio_segment: BioSegment,
        complete_fn: Option<BioCompleteFn>,
    ) -> Result<BioWaiter>;

    /// Returns the number of pages in the backend.
    fn npages(&self) -> usize;

    /// Reads a page from the backend asynchronously.
    fn read_page_async(
        &self,
        idx: usize,
        locked_page: cache_page::LockedCachePage,
    ) -> Result<BioWaiter> {
        let bio_segment = BioSegment::new_from_segment(
            Segment::from(locked_page.deref().clone()).into(),
            BioDirection::FromDevice,
        );

        let complete_fn: Box<dyn FnOnce(bool) + Send + Sync> = Box::new(move |success| {
            if success {
                locked_page.set_up_to_date();
            }
            // The page lock is released when `locked_page` (LockedCachePage) is dropped here.
        });

        self.submit_read_io(idx, bio_segment, Some(complete_fn))
    }

    /// Writes a page to the backend asynchronously.
    fn write_page_async(
        &self,
        idx: usize,
        locked_page: cache_page::LockedCachePage,
    ) -> Result<BioWaiter> {
        let bio_segment = BioSegment::alloc(1, BioDirection::ToDevice);
        bio_segment
            .writer()
            .unwrap()
            .write_fallible(&mut locked_page.reader().to_fallible())?;

        let page = locked_page.unlock();
        let submit_page = page.clone();

        let complete_fn: Box<dyn FnOnce(bool) + Send + Sync> = Box::new(move |success| {
            cache_page::clear_writing_back(&submit_page);
            if !success {
                // TODO: Record the writeback error (e.g., EIO) in the VMO
                // (or the corresponding inode) so that a subsequent sync syscall
                // can detect and report it to userspace.
                //
                // Following Linux's design, we intentionally do **not** re-dirty the
                // page here. Re-dirtying would cause the writeback mechanism to retry
                // the I/O indefinitely, which could stall the entire system if the
                // underlying device has a persistent hardware fault. Instead, the page
                // is left clean and the data is considered lost.
                log::error!("Writeback I/O failed for page index {idx}; data may be lost");
            }
        });

        let res = self.submit_write_io(idx, bio_segment, Some(complete_fn));
        if res.is_err() {
            // If submission fails, we need to clear the writing back state and re-dirty the page to
            //ensure the data will be retried on the next writeback attempt.
            let locked_page = page.lock();
            locked_page.set_dirty();
            cache_page::clear_writing_back(&locked_page);
        }

        res
    }
}

impl dyn PageCacheBackend {
    /// Reads a page from the backend synchronously.
    pub fn read_page(&self, idx: usize, page: cache_page::LockedCachePage) -> Result<()> {
        let waiter = self.read_page_async(idx, page)?;
        match waiter.wait() {
            Some(BioStatus::Complete) => Ok(()),
            _ => return_errno!(Errno::EIO),
        }
    }
}
