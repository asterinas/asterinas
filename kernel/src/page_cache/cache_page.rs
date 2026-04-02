// SPDX-License-Identifier: MPL-2.0

use core::{
    ops::Deref,
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use ostd::{
    impl_untyped_frame_meta_for,
    mm::{Frame, FrameAllocOptions, HasPaddr},
    sync::WaitQueue,
};

use crate::prelude::*;

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
    /// This bit is set and normally checked while holding the page lock. It is
    /// cleared without the page lock only from the BIO completion callback after
    /// the VMO writeback path has handed off the writeback state.
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
    ///
    /// This method may block. It must not be called while holding a spinlock or
    /// with interrupts disabled.
    fn lock(self) -> LockedCachePage;

    /// Ensures the page is initialized, calling `init_fn` if necessary.
    ///
    /// This method may block while waiting for another initializer. It must not
    /// be called while holding a spinlock or with interrupts disabled.
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
            PageState::Uninit | PageState::PendingWrite
        )
    }

    /// Checks if the page is up-to-date.
    fn is_up_to_date(&self) -> bool {
        matches!(
            self.metadata().state.load(Ordering::Acquire),
            PageState::UpToDate
        )
    }

    /// Checks if the page is dirty.
    fn is_dirty(&self) -> bool {
        matches!(
            self.metadata().state.load(Ordering::Acquire),
            PageState::Dirty
        )
    }

    /// Checks if the page is reserved for a full-page overwrite.
    fn is_pending_write(&self) -> bool {
        matches!(
            self.metadata().state.load(Ordering::Acquire),
            PageState::PendingWrite
        )
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
        wait_queue.wait_until(|| {
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

        let mut locked_page = self.clone().lock();
        // Check again after acquiring the lock to avoid duplicate initialization.
        if !locked_page.is_uninit() {
            return Ok(());
        }

        if locked_page.is_pending_write() {
            let page = locked_page.unlock();
            self.wait_queue()
                .wait_until(|| (!self.is_pending_write()).then_some(()));

            if !self.is_uninit() {
                return Ok(());
            }

            // Writing for pending-write pages failed, the page is still uninitialized
            // and we can proceed to initialize it.
            locked_page = page.lock();
        }

        init_fn(locked_page)
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
    pub(super) fn unlock(mut self) -> CachePage {
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
    pub(super) fn set_up_to_date(&self) {
        self.page()
            .metadata()
            .state
            .store(PageState::UpToDate, Ordering::Release);
    }

    /// Marks the page as dirty.
    ///
    /// This indicates that the page has been modified and needs to be
    /// written back to disk eventually.
    pub(super) fn set_dirty(&self) {
        self.metadata()
            .state
            .store(PageState::Dirty, Ordering::Release);
    }

    /// Marks the page as reserved for an in-progress full-page overwrite.
    pub(super) fn set_pending_write(&self) {
        self.metadata()
            .state
            .store(PageState::PendingWrite, Ordering::Release);
    }

    /// Clears the pending write state of the page.
    ///
    /// This happens when the full-page overwrite fails (e.g., due to an I/O error),
    /// and the page remains uninitialized and available for other writers to initialize it.
    pub(super) fn clear_pending_write(&self) {
        self.metadata()
            .state
            .store(PageState::Uninit, Ordering::Release);
    }

    /// Sets the writing back flag of the page, indicating that the page
    /// is in-flight to storage.
    pub(super) fn set_writing_back(&self) {
        self.metadata()
            .is_writing_back
            .store(true, Ordering::Release);
    }

    /// Waits until the page finishes writing back to storage.
    ///
    /// This function will wait on the same wait queue used for locking the page.
    pub(super) fn wait_until_finish_writing_back(&self) {
        self.wait_queue
            .wait_until(|| (!self.is_writing_back()).then_some(()));
    }

    /// Checks if the page is currently being written back to storage.
    pub(super) fn is_writing_back(&self) -> bool {
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
pub(super) enum PageState {
    /// `Uninit` indicates a new allocated page which content has not been initialized.
    /// The page is available to write, not available to read.
    Uninit = 0,
    /// `PendingWrite` indicates a page reserved for a full-page overwrite.
    /// Readers and read-modify-write paths must wait for the writer to finish
    /// instead of reading the backend.
    PendingWrite = 1,
    /// `UpToDate` indicates a page which content is consistent with corresponding disk content.
    /// The page is available to read and write.
    UpToDate = 2,
    /// `Dirty` indicates a page which content has been updated and not written back to underlying disk.
    /// The page is available to read and write.
    Dirty = 3,
}

impl TryFrom<u8> for PageState {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::Uninit),
            1 => Ok(Self::PendingWrite),
            2 => Ok(Self::UpToDate),
            3 => Ok(Self::Dirty),
            _ => return_errno_with_message!(Errno::EINVAL, "invalid page state"),
        }
    }
}

impl From<PageState> for u8 {
    fn from(value: PageState) -> Self {
        value as u8
    }
}

define_atomic_version_of_integer_like_type!(PageState, try_from = true, {
    #[derive(Debug)]
    struct AtomicPageState(AtomicU8);
});

pub(super) fn clear_writing_back(page: &CachePage) {
    page.metadata()
        .is_writing_back
        .store(false, Ordering::Release);
    page.wait_queue().wake_all();
}
