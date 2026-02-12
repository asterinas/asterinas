// SPDX-License-Identifier: MPL-2.0

use core::{
    ops::{Deref, Range},
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
};

use aster_block::bio::{BioCompleteFn, BioDirection, BioSegment, BioStatus, BioWaiter};
use ostd::{
    impl_untyped_frame_meta_for,
    mm::{Frame, FrameAllocOptions, HasPaddr, Segment, io_util::HasVmReaderWriter},
    sync::WaitQueue,
};

use crate::{prelude::*, vm::vmo::Vmo};

/// The page cache type.
///
/// The page cache is implemented using a [`Vmo`]. Typically, a page cache for
/// a disk-based file system (e.g., ext2, exfat) is a **disk-backed VMO**, which
/// is associated with a [`PageCacheBackend`] that provides I/O operations to read
/// from and write to the underlying block device. In contrast, for purely in-memory
/// file systems (e.g., ramfs), the page cache is an **anonymous VMO** — it has no
/// backend and its pages exist only in RAM.
pub type PageCache = Arc<Vmo>;

/// A trait for page cache operations.
///
/// The page cache serves as an in-memory buffer between the file system and
/// block devices, caching frequently accessed file data to improve performance.
pub trait PageCacheOps {
    /// Creates a new page cache with the specified capacity.
    fn with_capacity(capacity: usize, backend: Weak<dyn PageCacheBackend>) -> Result<Arc<Self>>;

    /// Resizes the page cache to the target size.
    ///
    /// The `new_size` will be rounded up to page boundaries. If the new size is smaller
    /// than the current size, pages that fall entirely within the truncated range will be
    /// decommitted (freed). For the page that is only partially truncated (i.e., the page
    /// containing the new boundary), the truncated portion will be filled with zeros instead.
    ///
    /// The `old_size` represents the actual used range of the page cache (i.e., the logical
    /// size of the cached content), which may differ from the total capacity of the page cache.
    /// It is used to determine the boundary of the previously valid data so that only the
    /// discarded logical range (from `new_size` to `old_size`) within a partially truncated
    /// page needs to be zero-filled.
    fn resize(&self, new_size: usize, old_size: usize) -> Result<()>;

    /// Flushes the dirty pages in the specified range to the backend storage.
    ///
    /// This operation ensures that any modifications made to the pages within the given
    /// range are persisted to the underlying storage device or file system.
    ///
    /// If the given range exceeds the current size of the page cache, only the pages within
    /// the valid range will be flushed.
    fn flush_range(&self, range: Range<usize>) -> Result<()>;

    /// Discards the pages within the specified range from the page cache.
    ///
    /// This operation will first **flush** the dirty pages in the range to the backend storage,
    /// ensuring that any modifications are persisted. After flushing, the pages are removed
    /// from the page cache. This is useful for invalidating cached data that is no longer needed
    /// or has become stale.
    fn discard_range(&self, range: Range<usize>) -> Result<()>;

    /// Fills the specified range of the page cache with zeros.
    fn fill_zeros(&self, range: Range<usize>) -> Result<()>;
}

/// A page in the page cache.
pub type CachePage = Frame<CachePageMeta>;

/// Metadata for a page in the page cache.
#[derive(Debug)]
pub struct CachePageMeta {
    /// The current state of the page (uninit, up-to-date, or dirty).
    state: AtomicPageState,
    /// This bit acts as a mutex for the corresponding page.
    ///
    /// When this bit is set, the holder has the exclusive right to perform critical
    /// state transitions (e.g., preparing for I/O).
    lock: AtomicBool,
    /// This bit indicates that the page's contents are "in-flight" to storage.
    ///
    /// This bit works like `PG_writeback` in Linux, it helps the page cache
    /// avoid holding the page lock for an extended period during writeback to the backend.
    ///
    /// The setting and checking of this bit must be performed while holding the lock.
    is_writing_back: AtomicBool,
    // TODO: Add a reverse mapping from the page to VMO for eviction.
}

impl Default for CachePageMeta {
    fn default() -> Self {
        Self {
            state: AtomicPageState::new(PageState::Uninit),
            lock: AtomicBool::new(false),
            is_writing_back: AtomicBool::new(false),
        }
    }
}

impl_untyped_frame_meta_for!(CachePageMeta);

pub trait CachePageExt: Sized {
    /// Gets the metadata associated with the cache page.
    fn metadata(&self) -> &CachePageMeta;

    /// Gets the wait queue associated with the cache page.
    fn wait_queue(&self) -> &'static WaitQueue;

    /// Tries to lock the cache page.
    fn try_lock(&self) -> Option<LockedCachePage>;

    /// Locks the cache page, blocking until the lock is acquired.
    fn lock(self) -> LockedCachePage;

    /// Ensures the page is initialized, calling `init_fn` if necessary.
    fn ensure_init(&self, init_fn: impl Fn(LockedCachePage) -> Result<()>) -> Result<()>;

    /// Allocates a new cache page which content and state are uninitialized.
    fn alloc_uninit() -> Result<CachePage> {
        let meta = CachePageMeta::default();
        let page = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_frame_with(meta)?;
        Ok(page)
    }

    /// Allocates a new zeroed cache page with the up-to-date state.
    fn alloc_zero() -> Result<CachePage> {
        let meta = CachePageMeta {
            state: AtomicPageState::new(PageState::UpToDate),
            lock: AtomicBool::new(false),
            is_writing_back: AtomicBool::new(false),
        };
        let page = FrameAllocOptions::new()
            .zeroed(true)
            .alloc_frame_with(meta)?;
        Ok(page)
    }

    /// Checks if the page is uninitialized.
    fn is_uninit(&self) -> bool {
        matches!(
            self.metadata().state.load(Ordering::Acquire),
            PageState::Uninit
        )
    }

    /// Checks if the page is dirty.
    fn is_dirty(&self) -> bool {
        matches!(
            self.metadata().state.load(Ordering::Acquire),
            PageState::Dirty
        )
    }

    /// Clears the writing back flag of the page.
    fn clear_writing_back(&self) {
        self.metadata()
            .is_writing_back
            .store(false, Ordering::Release);
        self.wait_queue().wake_all();
    }
}

impl CachePageExt for CachePage {
    fn metadata(&self) -> &CachePageMeta {
        self.meta()
    }

    fn wait_queue(&self) -> &'static WaitQueue {
        const PAGE_SHIFT: u32 = PAGE_SIZE.trailing_zeros();
        const PAGE_WAIT_QUEUE_MASK: usize = 0xff;
        const PAGE_WAIT_QUEUE_NUM: usize = PAGE_WAIT_QUEUE_MASK + 1;

        /// Global array of wait queues for page cache operations.
        ///
        /// Each wait queue in this array handles wait/wake operations for a subset of cache pages.
        /// The queue for a specific page is selected using: `PAGE_WAIT_QUEUES[page.paddr() & PAGE_WAIT_QUEUE_MASK]`.
        ///
        /// This approach avoids the overhead of per-page wait queues while still providing
        /// reasonable concurrency through hashing.
        static PAGE_WAIT_QUEUES: [WaitQueue; PAGE_WAIT_QUEUE_NUM] =
            [const { WaitQueue::new() }; PAGE_WAIT_QUEUE_NUM];

        &PAGE_WAIT_QUEUES[(self.paddr() >> PAGE_SHIFT) & PAGE_WAIT_QUEUE_MASK]
    }

    fn try_lock(&self) -> Option<LockedCachePage> {
        let wait_queue = self.wait_queue();
        self.metadata()
            .lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
            .then(|| LockedCachePage::new(self.clone(), wait_queue))
    }

    fn lock(self) -> LockedCachePage {
        let wait_queue = self.wait_queue();
        self.wait_queue().wait_until(|| {
            self.metadata()
                .lock
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .ok()
        });
        LockedCachePage::new(self, wait_queue)
    }

    fn ensure_init(&self, init_fn: impl Fn(LockedCachePage) -> Result<()>) -> Result<()> {
        // Fast path: if the page is already initialized, return immediately without waiting.
        if !self.is_uninit() {
            return Ok(());
        }

        let lock_page = self.clone().lock();
        // Check again after acquiring the lock to avoid duplicate initialization.
        if !lock_page.is_uninit() {
            return Ok(());
        }

        init_fn(lock_page)
    }
}

/// A locked cache page.
///
/// The locked page has the exclusive right to perform critical
/// state transitions (e.g., preparing for I/O).
pub struct LockedCachePage {
    page: Option<CachePage>,
    wait_queue: &'static WaitQueue,
}

impl Debug for LockedCachePage {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("LockedCachePage")
            .field("page", &self.page)
            .finish()
    }
}

impl LockedCachePage {
    fn new(page: CachePage, wait_queue: &'static WaitQueue) -> Self {
        Self {
            page: Some(page),
            wait_queue,
        }
    }

    /// Unlocks the page and returns the underlying cache page.
    pub fn unlock(mut self) -> CachePage {
        let page = self.page.take().expect("page already taken");
        page.metadata().lock.store(false, Ordering::Release);
        self.wait_queue.wake_all();
        page
    }

    fn page(&self) -> &CachePage {
        self.page.as_ref().expect("page already taken")
    }

    /// Marks the page as up-to-date.
    ///
    /// This indicates that the page's contents are synchronized with disk
    /// and can be safely read.
    pub fn set_up_to_date(&self) {
        self.page()
            .metadata()
            .state
            .store(PageState::UpToDate, Ordering::Release);
    }

    /// Marks the page as dirty.
    ///
    /// This indicates that the page has been modified and needs to be
    /// written back to disk eventually.
    pub fn set_dirty(&self) {
        self.metadata()
            .state
            .store(PageState::Dirty, Ordering::Release);
    }

    /// Sets the writing back flag of the page, indicating that the page
    /// is in-flight to storage.
    pub fn set_writing_back(&self) {
        self.metadata()
            .is_writing_back
            .store(true, Ordering::Release);
    }

    /// Waits until the page finishes writing back to storage.
    ///
    /// This function will wait on the same wait queue used for locking the page.
    fn wait_until_finish_writing_back(&self) {
        self.wait_queue
            .wait_until(|| (!self.is_writing_back()).then_some(()));
    }

    /// Checks if the page is currently being written back to storage.
    fn is_writing_back(&self) -> bool {
        self.metadata().is_writing_back.load(Ordering::Acquire)
    }
}

impl Deref for LockedCachePage {
    type Target = CachePage;

    fn deref(&self) -> &Self::Target {
        self.page.as_ref().expect("page already taken")
    }
}

impl Drop for LockedCachePage {
    fn drop(&mut self) {
        if let Some(page) = &self.page {
            page.metadata().lock.store(false, Ordering::Release);
            self.wait_queue.wake_all();
        }
    }
}

/// The state of a page in the page cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PageState {
    /// `Uninit` indicates a new allocated page which content has not been initialized.
    /// The page is available to write, not available to read.
    Uninit = 0,
    /// `UpToDate` indicates a page which content is consistent with corresponding disk content.
    /// The page is available to read and write.
    UpToDate = 1,
    /// `Dirty` indicates a page which content has been updated and not written back to underlying disk.
    /// The page is available to read and write.
    Dirty = 2,
}

/// A page state with atomic operations.
#[derive(Debug)]
pub struct AtomicPageState {
    state: AtomicU8,
}

impl AtomicPageState {
    pub fn new(state: PageState) -> Self {
        Self {
            state: AtomicU8::new(state as _),
        }
    }

    pub fn load(&self, order: Ordering) -> PageState {
        let val = self.state.load(order);
        match val {
            0 => PageState::Uninit,
            1 => PageState::UpToDate,
            2 => PageState::Dirty,
            _ => unreachable!(),
        }
    }

    pub fn store(&self, val: PageState, order: Ordering) {
        self.state.store(val as u8, order);
    }
}

/// This trait represents the backend for the page cache.
///
/// Implementors only need to provide the raw I/O operations (`read_page_raw` and
/// `write_page_raw`). The trait provides default implementations for `read_page_async`
/// and `write_page_async` that automatically manage `LockedCachePage` state transitions
/// (setting up-to-date on read success, clearing dirty on write success, etc.).
pub trait PageCacheBackend: Sync + Send {
    /// Submits a raw read I/O for the page at the given index.
    ///
    /// The `bio_segment` is the target memory to read into, and `complete_fn` should
    /// be passed to the block device's async read API. The implementor should **not**
    /// manage page state — that is handled by the default `read_page_async`.
    fn read_page_raw(
        &self,
        idx: usize,
        bio_segment: BioSegment,
        complete_fn: Option<BioCompleteFn>,
    ) -> Result<BioWaiter>;

    /// Submits a raw write I/O for the page at the given index.
    ///
    /// The `bio_segment` is the source memory to write from, and `complete_fn` should
    /// be passed to the block device's async write API. The implementor should **not**
    /// manage page state — that is handled by the default `write_page_async`.
    fn write_page_raw(
        &self,
        idx: usize,
        bio_segment: BioSegment,
        complete_fn: Option<BioCompleteFn>,
    ) -> Result<BioWaiter>;

    /// Returns the number of pages in the backend.
    fn npages(&self) -> usize;

    /// Reads a page from the backend asynchronously.
    fn read_page_async(&self, idx: usize, frame: LockedCachePage) -> Result<BioWaiter> {
        let bio_segment = BioSegment::new_from_segment(
            Segment::from(frame.deref().clone()).into(),
            BioDirection::FromDevice,
        );

        let complete_fn: Box<dyn FnOnce(bool) + Send + Sync> = Box::new(move |success| {
            if success {
                frame.set_up_to_date();
            }
            // The page lock is released when `frame` (LockedCachePage) is dropped here.
        });

        self.read_page_raw(idx, bio_segment, Some(complete_fn))
    }

    /// Writes a page to the backend asynchronously.
    fn write_page_async(&self, idx: usize, frame: LockedCachePage) -> Result<BioWaiter> {
        let bio_segment = BioSegment::alloc(1, BioDirection::ToDevice);
        bio_segment
            .writer()
            .unwrap()
            .write_fallible(&mut frame.reader().to_fallible())?;

        frame.wait_until_finish_writing_back();
        frame.set_writing_back();
        frame.set_up_to_date();
        let frame = frame.unlock();

        let complete_fn: Box<dyn FnOnce(bool) + Send + Sync> = Box::new(move |success| {
            frame.clear_writing_back();
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

        self.write_page_raw(idx, bio_segment, Some(complete_fn))
    }
}

impl dyn PageCacheBackend {
    /// Reads a page from the backend synchronously.
    pub fn read_page(&self, idx: usize, page: LockedCachePage) -> Result<()> {
        let waiter = self.read_page_async(idx, page)?;
        match waiter.wait() {
            Some(BioStatus::Complete) => Ok(()),
            _ => return_errno!(Errno::EIO),
        }
    }

    /// Writes a page to the backend synchronously.
    pub fn write_page(&self, idx: usize, page: LockedCachePage) -> Result<()> {
        let waiter = self.write_page_async(idx, page)?;
        match waiter.wait() {
            Some(BioStatus::Complete) => Ok(()),
            _ => return_errno!(Errno::EIO),
        }
    }
}
