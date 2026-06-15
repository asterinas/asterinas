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
//! The subsystem is split into four layers with different responsibilities:
//!
//! - [`PageCache`] is the per-file façade used by filesystems. It exposes
//!   buffered I/O, resize, flush, and invalidation operations in filesystem
//!   terms.
//! - [`Vmo`] is the lower-level memory object underneath a page cache. It owns
//!   the page array, commits pages on demand, and is the abstraction shared
//!   with page-fault and mapping code.
//! - [`CachePage`] stores cached page contents, while
//!   [`cache_page::PageState`] records whether those contents are uninitialized,
//!   clean, or dirty.
//! - [`PageCacheBackend`] is the storage-facing contract used
//!   to make a page readable or durable. [`BlockAsPageCacheBackend`] is a
//!   helper interface for backends that can satisfy that contract with
//!   block-device BIOs.
//!
//! For a file with a backend, the steady-state flow is
//! `filesystem metadata -> PageCache -> Vmo -> PageCacheBackend -> block
//! device or remote server`. Anonymous page caches use the same `PageCache` / `Vmo` layers
//! without a backend.
//!
//! # Responsibility Boundary
//!
//! `PageCache` manages page-aligned cache capacity and cached contents. The
//! filesystem still owns file size, extent or block mapping metadata, and the
//! higher-level locking that keeps those decisions stable while the page cache
//! is accessed.
//!
//! Stay at [`PageCache`] when the caller is operating in filesystem terms. Drop
//! to [`Vmo`] only when code needs the lower-level memory-object interface
//! directly, such as `mmap` setup or page-fault handling.
//!
//! # Synchronization Model
//!
//! The page-cache subsystem serializes per-page state transitions, including
//! the backend page states in [`cache_page::PageState`] and the auxiliary
//! writeback tracking bit in [`CachePageMeta`]. It does not provide whole-file
//! serialization or page-table quiescence.
//!
//! Callers still need higher-level synchronization in two buckets:
//!
//! - **Filesystem metadata / buffered-I/O lock.** Hold the inode- or file-level
//!   lock that stabilizes file size, extent metadata, and write
//!   ordering around filesystem-facing buffered access. The affected APIs are:
//!   - buffered reads through the [`VmIo`] implementation on [`PageCache`],
//!     such as [`VmIo::read_bytes`];
//!   - buffered writes through the [`VmIo`] implementation on [`PageCache`],
//!     such as [`VmIo::write_bytes`];
//!   - zero-filling writes through [`PageCache::fill_zeros`];
//!   - standalone [`PageCache::flush_range`] calls when `fsync`-like ordering
//!     matters; and
//!   - both sides of [`PageCache::resize`], because the file size and
//!     page-cache capacity must be updated in one critical section.
//! - **Reverse-mapping / invalidation lock.** Hold the lock shared with the
//!   page-table and page-fault side whenever cached pages may disappear or the
//!   accessible VMO range may shrink. The affected APIs are:
//!   - shrinking [`PageCache::resize`];
//!   - [`PageCache::evict_range`];
//!   - [`PageCache::invalidate_range`]; and
//!   - mapping-side access through [`Vmo::commit_on`], [`Vmo::try_commit_page`]
//!     or [`Vmo::try_operate_on_range`] on the same file range.
//
// TODO: Implement built-in synchronization between mappings and [`PageCache`]
// to meet reverse-mapping constraints above.
// TODO: Remove the `VmIo` implementation for `PageCache` and make operations
// that need to be protected by upper locks accept `&mut self`, and modify the
// related usages.

use core::{
    ops::{Deref, Range},
    sync::atomic::Ordering,
};

use align_ext::AlignExt;
use aster_block::bio::{BioCompleteFn, BioDirection, BioSegment, BioStatus};
use io_util::batch::IoBatch;
use ostd::mm::{Segment, VmIo, VmIoFill, io::util::HasVmReaderWriter};

use crate::prelude::*;

mod cache_page;
#[cfg(ktest)]
mod tests;
mod vmo;

pub use cache_page::{CachePage, CachePageExt, CachePageMeta, LockedCachePage};
pub use vmo::{Vmo, VmoCommitError, VmoFlags, VmoOptions, WritableMappingStatus};

/// The page cache for a file-like object.
///
/// This is the abstraction a filesystem usually stores in an inode: it handles
/// buffered reads and writes, writeback, invalidation, and page-cache resizing,
/// while delegating per-page population and writeback to the underlying [`Vmo`].
///
/// A `PageCache` owns cached page contents and a page-aligned capacity. It does
/// not own:
///
/// - the filesystem's file size (EOF);
/// - extent or block-mapping metadata; or
/// - the higher-level synchronization that keeps metadata, buffered I/O,
///   invalidation, and VM mappings coherent.
///
/// See the module-level documentation of [`crate::vm::page_cache`], especially the
/// `Synchronization Model`, for the complete concurrency contract and the API
/// lists covered by the filesystem lock and the reverse-mapping /
/// invalidation lock.
///
/// Filesystems backed by persistent or remote storage create it with
/// [`PageCache::new_with_backend`] and a [`PageCacheBackend`]. Purely
/// in-memory filesystems can use [`PageCache::new_anon`] to get the same
/// buffered-I/O interface without a backend.
///
/// Reach for [`PageCache::as_vmo`] only when a lower-level consumer such as
/// memory mapping or page-fault code must operate on the underlying [`Vmo`]
/// directly.
#[derive(Clone, Debug)]
pub struct PageCache(Arc<Vmo>);

impl PageCache {
    /// Creates a page cache with a backend and the specified initial capacity
    /// in bytes.
    pub fn new_with_backend(size: usize, backend: Weak<dyn PageCacheBackend>) -> Result<Self> {
        Ok(Self::from(
            VmoOptions::new_page_cache(size, backend).alloc()?,
        ))
    }

    /// Creates an anonymous page cache with the specified initial capacity in
    /// bytes.
    pub fn new_anon(size: usize) -> Result<Self> {
        Ok(Self::from(VmoOptions::new_anon(size).alloc()?))
    }

    /// Returns the wrapped [`Vmo`].
    pub fn as_vmo(&self) -> &Arc<Vmo> {
        &self.0
    }

    /// Returns the current page-cache capacity in bytes.
    ///
    /// This size is page-aligned and may exceed the file size. The filesystem
    /// remains responsible for tracking EOF separately.
    pub fn size(&self) -> usize {
        self.0.size()
    }

    /// Returns the writable mapping status of the underlying VMO.
    pub fn writable_mapping_status(&self) -> &WritableMappingStatus {
        self.0.writable_mapping_status()
    }

    /// Resizes the page-cache capacity to cover a new file size.
    ///
    /// `new_file_size` is the post-resize file size requested by the filesystem.
    /// The underlying cache capacity is rounded up to page boundaries. If the
    /// new size is smaller than the current size, pages that fall entirely
    /// within the truncated range will be decommitted (freed). For the page
    /// that is only partially truncated (i.e., the page containing the new
    /// boundary), the truncated portion will be filled with zeros instead.
    ///
    /// The `old_file_size` must be the file length before this resize. It is
    /// used to determine the boundary of previously valid data so that only the
    /// discarded range within a partially truncated tail page is zero-filled.
    ///
    /// Extending the page cache does not eagerly allocate pages and therefore
    /// cannot return an error. Shrinking may fail because it can zero-fill or
    /// decommit existing pages.
    ///
    /// # Size Synchronization
    ///
    /// `PageCache::resize` only updates the page-aligned [`Vmo`] capacity. The
    /// filesystem must keep that capacity synchronized with its own file size
    /// under the same inode- or file-level lock that excludes conflicting
    /// buffered I/O, page faults, and invalidation.
    ///
    /// The required ordering is:
    ///
    /// - When extending a file, update the file size before increasing
    ///   [`Vmo::size`] so subsequent reads can observe the new range.
    /// - When truncating a file, shrink [`Vmo::size`] before decreasing the
    ///   file size so reads beyond the new EOF cannot observe stale cached
    ///   pages.
    ///
    /// Accordingly, `old_file_size` must be the pre-resize file size captured
    /// inside that resize critical section.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ostd::mm::PAGE_SIZE;
    ///
    /// use crate::vm::page_cache::PageCache;
    ///
    /// let page_cache = PageCache::new_anon(0).unwrap();
    ///
    /// // Extend: publish the new file size first, then grow the page cache
    /// // with the previous file size.
    /// let mut file_size = PAGE_SIZE + 123;
    /// page_cache.resize(file_size, 0).unwrap();
    /// assert_eq!(page_cache.size(), 2 * PAGE_SIZE);
    ///
    /// // Truncate: shrink the page cache first, while passing the old file
    /// // size captured in the same critical section.
    /// let old_file_size = file_size;
    /// file_size = 512;
    /// page_cache.resize(file_size, old_file_size).unwrap();
    /// assert_eq!(page_cache.size(), PAGE_SIZE);
    /// ```
    //
    // TODO: Integrate a reverse-mapping lock or equivalent synchronization so
    // shrink/truncate can coordinate with mapped pages and concurrent page
    // faults.
    pub fn resize(&self, new_file_size: usize, old_file_size: usize) -> Result<()> {
        let vmo = &self.0;
        assert!(vmo.flags.contains(VmoFlags::RESIZABLE));

        if new_file_size < old_file_size && !new_file_size.is_multiple_of(PAGE_SIZE) {
            let fill_zero_end = old_file_size.min(new_file_size.align_up(PAGE_SIZE));
            self.fill_zeros(new_file_size..fill_zero_end)?;
        } else if new_file_size > old_file_size && !old_file_size.is_multiple_of(PAGE_SIZE) {
            let fill_zero_end = new_file_size.min(old_file_size.align_up(PAGE_SIZE));
            self.fill_zeros(old_file_size..fill_zero_end)?;
        }

        let new_cache_size = new_file_size.align_up(PAGE_SIZE);
        let locked_pages = vmo.pages.lock();
        let old_cache_size = vmo.size();
        if new_cache_size == old_cache_size {
            return Ok(());
        }

        vmo.size.store(new_cache_size, Ordering::Release);

        if new_cache_size < old_cache_size {
            let decommit_range = new_cache_size..old_cache_size;
            if let Some(backed_vmo) = vmo.as_backed_vmo() {
                backed_vmo.decommit_pages(locked_pages, &decommit_range)?;
            } else {
                vmo.decommit_anon_pages(locked_pages, decommit_range)?;
            }
        }

        Ok(())
    }

    /// Flushes dirty pages in the specified range to the backend storage.
    ///
    /// This walks the current cache contents, submits writeback for pages that
    /// are dirty when this pass reaches them, and waits for the submitted I/O
    /// to complete.
    ///
    /// Filesystems that need `fsync`-like guarantees must still exclude
    /// concurrent writers or repeat the operation until their own ordering
    /// requirements are met.
    ///
    /// Callers must hold the filesystem-level lock that serializes operations in
    /// the target range.
    ///
    /// If the given range exceeds the current size of the page cache, only the pages within
    /// the valid range will be flushed.
    pub fn flush_range(&self, range: Range<usize>) -> Result<()> {
        let Some(vmo) = self.0.as_backed_vmo() else {
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
    /// Callers must hold the filesystem-level lock that serializes operations in
    /// the target range.
    ///
    /// Because this method can detach pages from the cache, callers must also
    /// hold the reverse-mapping / page-table invalidation lock that excludes
    /// concurrent page faults and mapped access to the same range.
    //
    // TODO: Integrate a reverse-mapping lock or equivalent synchronization so
    // eviction can coordinate with mapped pages and concurrent page faults
    // before removing cached pages from the page cache.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub fn evict_range(&self, range: Range<usize>) -> Result<()> {
        let Some(vmo) = self.0.as_backed_vmo() else {
            return Ok(());
        };

        vmo.evict_up_to_date_pages(&range)
    }

    /// Flushes dirty pages and then evicts clean pages in the specified range.
    ///
    /// This is the standard preparation step before issuing direct I/O that must
    /// bypass the page cache. It uses the same locking requirements as
    /// [`PageCache::flush_range`] and [`PageCache::evict_range`].
    pub fn invalidate_range(&self, range: Range<usize>) -> Result<()> {
        let Some(vmo) = self.0.as_backed_vmo() else {
            return Ok(());
        };

        vmo.flush_dirty_pages(&range)?;
        vmo.evict_up_to_date_pages(&range)
    }

    /// Fills the specified range of the page cache with zeros.
    ///
    /// Callers must hold the filesystem-level lock that serializes operations in
    /// the target range.
    pub fn fill_zeros(&self, range: Range<usize>) -> Result<()> {
        self.0.fill_zeros(range)
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

/// A storage backend for a backed [`PageCache`].
///
/// This trait is the high-level contract used by the page-cache layer to load
/// page contents from storage and write dirty contents back. A successful read
/// makes the requested page usable by readers; a successful write makes the
/// page's current contents durable in the backend.
///
/// Direct implementations are useful for storage that does not naturally expose
/// block-device BIOs, such as network filesystems. Filesystems backed by block
/// devices usually implement [`BlockAsPageCacheBackend`] instead.
pub trait PageCacheBackend: Sync + Send {
    /// Reads a page from the backend asynchronously.
    ///
    /// The caller may try to pass an index that exceeds the size of the
    /// underlying backend (e.g., the file size). This can occur if file
    /// truncation and a page fault occur at the same time. If this happens,
    /// this method should fail with `EINVAL`.
    fn read_page_async(
        &self,
        idx: usize,
        locked_page: LockedCachePage,
        io_batch: &mut IoBatch,
    ) -> Result<()>;

    /// Writes a page to the backend asynchronously.
    ///
    /// If the caller tries to pass an index that exceeds the size of the
    /// underlying backend (e.g., the file size), this method should fail with
    /// `EINVAL`. Note that this cannot happen until we support concurrent file
    /// truncation and page writeback.
    //
    // TODO: Revise this behavior once file truncation and page writeback can
    // happen concurrently.
    fn write_page_async(
        &self,
        idx: usize,
        locked_page: LockedCachePage,
        io_batch: &mut IoBatch,
    ) -> Result<()>;
}

impl dyn PageCacheBackend {
    /// Reads a page from the backend synchronously.
    pub fn read_page(&self, idx: usize, page: LockedCachePage) -> Result<()> {
        let mut io_batch = IoBatch::with_capacity(1);
        self.read_page_async(idx, page, &mut io_batch)?;
        io_batch.wait_all()?;
        Ok(())
    }
}

/// A block-I/O object that can serve as a [`PageCacheBackend`].
///
/// This trait is a convenience layer for filesystems whose cached file data is
/// served by block I/O. Implementors submit the supplied [`BioSegment`] for the
/// page at the requested index.
///
/// The blanket [`PageCacheBackend`] implementation adapts this trait to the
/// generic page-cache backend contract.
///
/// The `complete_fn` passed to submit methods may run from interrupt context,
/// so implementations must not allocate, take blocking locks, or hold a lock
/// that a waiter on the page wait queue may already hold.
//
// TODO: This trait should provide interfaces for reading or writing multiple
// pages in a single BIO to improve efficiency for sequential I/O.
pub trait BlockAsPageCacheBackend: Sync + Send {
    /// Submits read I/O for the page at `idx`.
    ///
    /// `bio_segment` identifies the page memory that must receive the data.
    /// Implementations must attach `complete_fn` to the underlying
    /// asynchronous I/O so the page-cache layer can mark successful
    /// reads up to date and release the page lock.
    ///
    /// If the page only contains zeros, implementations may call `complete_fn`
    /// with [`BioStatus::Zeros`], skip I/O, and leave `bio_segment` unfilled.
    ///
    /// Implementations should fail with `EINVAL` for an out-of-bounds `idx`.
    /// See also [`PageCacheBackend::read_page_async`].
    fn submit_read_bio(
        &self,
        idx: usize,
        bio_segment: BioSegment,
        complete_fn: BioCompleteFn,
        io_batch: &mut IoBatch,
    ) -> Result<()>;

    /// Submits write I/O for the page at `idx`.
    ///
    /// `bio_segment` contains the stable page snapshot that must be written.
    /// Implementations must attach `complete_fn` to the underlying
    /// asynchronous I/O so the page-cache layer can finish writeback
    /// bookkeeping and report failures.
    ///
    /// Implementations should fail with `EINVAL` for an out-of-bounds `idx`.
    /// See also [`PageCacheBackend::write_page_async`].
    fn submit_write_bio(
        &self,
        idx: usize,
        bio_segment: BioSegment,
        complete_fn: BioCompleteFn,
        io_batch: &mut IoBatch,
    ) -> Result<()>;
}

impl<T: BlockAsPageCacheBackend> PageCacheBackend for T {
    fn read_page_async(
        &self,
        idx: usize,
        locked_page: LockedCachePage,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        let bio_segment = BioSegment::new_from_segment(
            Segment::from(locked_page.deref().clone()).into(),
            BioDirection::FromDevice,
        );

        let complete_fn: BioCompleteFn = Box::new(move |status| {
            if status == BioStatus::Zeros {
                locked_page.fill_zeros(0, PAGE_SIZE).unwrap();
                locked_page.set_up_to_date();
            } else if status == BioStatus::Complete {
                locked_page.set_up_to_date();
            }
            // The page lock is released when `locked_page` (LockedCachePage) is dropped here.
        });

        self.submit_read_bio(idx, bio_segment, complete_fn, io_batch)
    }

    fn write_page_async(
        &self,
        idx: usize,
        locked_page: LockedCachePage,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        locked_page.wait_until_finish_writing_back();

        let bio_segment = BioSegment::alloc(1, BioDirection::ToDevice);
        bio_segment
            .writer()
            .unwrap()
            .write(&mut locked_page.reader());

        locked_page.set_writing_back();
        locked_page.set_up_to_date();

        let page = locked_page.unlock();
        let submit_page = page.clone();

        let complete_fn: BioCompleteFn = Box::new(move |status| {
            submit_page.clear_writing_back();
            if status != BioStatus::Complete {
                // TODO: Record the writeback error (e.g., EIO) in the VMO
                // (or the corresponding inode) so that a subsequent sync syscall
                // can detect and report it to userspace.
                //
                // Following Linux's design, we intentionally do **not** re-dirty the
                // page here. Re-dirtying would cause the writeback mechanism to retry
                // the I/O indefinitely, which could stall the entire system if the
                // underlying device has a persistent hardware fault. Instead, the page
                // is left clean and the data is considered lost.
                ostd::error!(
                    "writeback I/O failed for page index {idx} with status {status:?}; data may be lost"
                );
            }
        });

        let res = self.submit_write_bio(idx, bio_segment, complete_fn, io_batch);
        if res.is_err() {
            // If submission fails, re-dirty the page so the next writeback can
            // retry the data that never reached the device queue.
            let locked_page = page.lock();
            locked_page.set_dirty();
            locked_page.clear_writing_back();
        }

        res
    }
}
