// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

use core::{
    ops::Deref,
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
};

use aster_block::bio::{BioStatus, BioWaiter};
use ostd::{
    impl_untyped_frame_meta_for,
    mm::{Frame, FrameAllocOptions, HasPaddr, UFrame, frame::meta::AnyFrameMeta},
    sync::WaitQueue,
};

use crate::{
    prelude::*,
    vm::vmo::{Vmo, VmoFlags, VmoOptions},
};

/// A page cache that manages in-memory copies of file data.
///
/// The page cache is backed by a [`Vmo`] and can optionally be associated with
/// a [`PageCacheBackend`] for file I/O operations.
pub struct PageCache {
    pages: Arc<Vmo>,
}

impl PageCache {
    /// Creates an empty page cache.
    ///
    /// The cache starts with zero size and can be resized later. If a `backend`
    /// is provided, the cache becomes file-backed; otherwise, it's anonymous.
    pub fn new(backend: Option<Arc<dyn PageCacheBackend>>) -> Result<Self> {
        let mut pages = VmoOptions::new(0).flags(VmoFlags::RESIZABLE);
        if let Some(backend) = backend {
            pages = pages.backend(backend)
        };

        let pages = pages.alloc()?;

        Ok(Self { pages })
    }

    /// Creates a page cache with an initial capacity.
    ///
    /// The `capacity` typically matches the size of the underlying file or storage.
    /// The cache can be resized later if needed.
    pub fn with_capacity(
        capacity: usize,
        backend: Option<Arc<dyn PageCacheBackend>>,
    ) -> Result<Self> {
        let mut pages = VmoOptions::new(capacity).flags(VmoFlags::RESIZABLE);
        if let Some(backend) = backend {
            pages = pages.backend(backend)
        };

        let pages = pages.alloc()?;

        Ok(Self { pages })
    }

    /// Returns a reference to the underlying VMO.
    pub fn pages(&self) -> &Arc<Vmo> {
        &self.pages
    }
}

impl Drop for PageCache {
    fn drop(&mut self) {
        // TODO:
        // The default destruction procedure exhibits slow performance.
        // In contrast, resizing the `VMO` to zero greatly accelerates the process.
        // We need to find out the underlying cause of this discrepancy.
        let _ = self.flush_range(0..self.pages.size());
    }
}

impl Debug for PageCache {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("PageCache")
            .field("size", &self.pages.size())
            .finish()
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

    /// Blocks until the page finishes writeback to disk.
    ///
    /// This is necessary before modifying a page that is currently being
    /// written back to ensure data consistency.
    pub fn wait_until_finish_write_back(&self) {
        self.wait_queue
            .wait_until(|| (!self.is_writing_back()).then_some(()));
    }

    /// Marks the page as up-to-date.
    ///
    /// This indicates that the page's contents are synchronized with disk
    /// and can be safely read.
    pub fn set_up_to_date(&self) {
        self.page()
            .metadata()
            .state
            .store(PageState::UpToDate, Ordering::Relaxed);
    }

    /// Marks the page as dirty.
    ///
    /// This indicates that the page has been modified and needs to be
    /// written back to disk eventually.
    pub fn set_dirty(&self) {
        self.metadata()
            .state
            .store(PageState::Dirty, Ordering::Relaxed);
    }

    /// Marks the page as being written back.
    ///
    /// This flag prevents concurrent modifications during the writeback operation.
    pub fn set_write_back(&self) {
        self.page()
            .metadata()
            .is_writing_back
            .store(true, Ordering::Relaxed);
    }

    /// Checks if the page is currently being written back.
    pub fn is_writing_back(&self) -> bool {
        self.page()
            .metadata()
            .is_writing_back
            .load(Ordering::Acquire)
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

/// A page in the page cache.
pub type CachePage = Frame<CachePageMeta>;

const PAGE_WAIT_QUEUE_MASK: usize = 0xff;
const PAGE_WAIT_QUEUE_NUM: usize = PAGE_WAIT_QUEUE_MASK + 1;

/// Global array of wait queues for page cache operations.
///
/// Each wait queue in this array handles wait/wake operations for a subset of cache pages.
/// The queue for a specific page is selected using: `PAGE_WAIT_QUEUES[page.paddr() & PAGE_WAIT_QUEUE_MASK]`.
///
/// Multiple operations can wait on the same queue:
/// - Waiting for a page to be unlocked (`lock`)
/// - Waiting for a page to be initialized (`wait_until_init`)
/// - Waiting for write-back to complete (`wait_until_finish_write_back`)
///
/// This approach avoids the overhead of per-page wait queues while still providing
/// reasonable concurrency through hashing.
static PAGE_WAIT_QUEUES: [WaitQueue; PAGE_WAIT_QUEUE_NUM] =
    [const { WaitQueue::new() }; PAGE_WAIT_QUEUE_NUM];

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
    /// Whether the page is currently being written back to disk.
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
    /// Tries to convert a untyped frame into a cache page.
    fn try_from_frame(frame: UFrame) -> Option<CachePage> {
        let frame: Frame<dyn AnyFrameMeta> = frame.into();
        frame.try_into().ok()
    }

    /// Gets the metadata associated with the cache page.
    fn metadata(&self) -> &CachePageMeta;

    /// Gets the wait queue associated with the cache page.
    fn wait_queue(&self) -> &'static WaitQueue;

    /// Tries to lock the cache page.
    fn try_lock(&self) -> Option<LockedCachePage>;

    /// Locks the cache page, blocking until the lock is acquired.
    fn lock(self) -> LockedCachePage;

    /// Waits until the page is initialized.
    fn wait_until_init(&self) {
        self.wait_queue()
            .wait_until(|| (self.load_state() != PageState::Uninit).then_some(()));
    }

    /// Allocates a new cache page which content and state are uninitialized.
    fn alloc_uninit() -> Result<CachePage> {
        let meta = CachePageMeta {
            state: AtomicPageState::new(PageState::Uninit),
            lock: AtomicBool::new(false),
            is_writing_back: AtomicBool::new(false),
        };
        let page = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_frame_with(meta)?;
        Ok(page)
    }

    /// Allocates a new zeroed cache page with the wanted state.
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

    /// Loads the current state of the cache page.
    fn load_state(&self) -> PageState {
        self.metadata().state.load(Ordering::Relaxed)
    }

    /// Clears the writeback flag and wakes waiting threads.
    fn clear_writing_back(&self) {
        self.metadata()
            .is_writing_back
            .store(false, Ordering::Release);
        self.wait_queue().wake_all();
    }

    /// Checks if the page is dirty.
    fn is_dirty(&self) -> bool {
        matches!(
            self.metadata().state.load(Ordering::Relaxed),
            PageState::Dirty
        )
    }
}

impl CachePageExt for CachePage {
    fn metadata(&self) -> &CachePageMeta {
        self.meta()
    }

    fn wait_queue(&self) -> &'static WaitQueue {
        &PAGE_WAIT_QUEUES[self.paddr() & PAGE_WAIT_QUEUE_MASK]
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
}

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
pub trait PageCacheBackend: Sync + Send {
    /// Reads a page from the backend asynchronously.
    fn read_page_async(&self, idx: usize, frame: LockedCachePage) -> Result<BioWaiter>;
    /// Writes a page to the backend asynchronously.
    fn write_page_async(&self, idx: usize, frame: LockedCachePage) -> Result<BioWaiter>;
    /// Returns the number of pages in the backend.
    fn npages(&self) -> usize;
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
