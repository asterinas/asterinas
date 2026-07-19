// SPDX-License-Identifier: MPL-2.0

use core::{
    borrow::Borrow,
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

/// The state of a page in the page cache.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PageState {
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

impl TryFrom<u8> for PageState {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::Uninit),
            1 => Ok(Self::UpToDate),
            2 => Ok(Self::Dirty),
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

/// Convenience operations on a [`CachePage`] handle.
///
/// Implemented for every [`CachePage`], this gives access to the page lock,
/// the per-page wait queue, and lifecycle helpers used during page commit.
pub trait CachePageExt: Sized {
    /// Gets the metadata associated with the cache page.
    fn metadata(&self) -> &CachePageMeta;

    /// Gets the wait queue associated with the cache page.
    fn wait_queue(&self) -> &'static WaitQueue;

    /// Tries to lock the cache page.
    #[expect(dead_code)]
    fn try_lock(self) -> Option<LockedCachePage>;

    /// Tries to lock the cache page by reference.
    fn try_lock_guard(&self) -> Option<LockedCachePageGuard<'_>>;

    /// Locks the cache page, blocking until the lock is acquired.
    ///
    /// This method may block. It must not be called while holding a spinlock or
    /// with interrupts disabled.
    fn lock(self) -> LockedCachePage;

    /// Locks the cache page by reference, blocking until the lock is acquired.
    ///
    /// This method may block. It must not be called while holding a spinlock or
    /// with interrupts disabled.
    fn lock_guard(&self) -> LockedCachePageGuard<'_>;

    /// Ensures the page is initialized, calling `init_fn` if necessary.
    ///
    /// This method may block while waiting for another initializer. It must not
    /// be called while holding a spinlock or with interrupts disabled.
    fn ensure_init(&self, init_fn: impl FnOnce(LockedCachePage) -> Result<()>) -> Result<()>;

    /// Clears writeback state for a page whose writeback has completed or was canceled.
    ///
    /// Callers must invoke this method only to finish a writeback operation
    /// previously started with [`LockedCachePage::set_writing_back`], either
    /// from its completion callback or while handling submission failure.
    fn clear_writing_back(&self) {
        self.metadata()
            .is_writing_back
            .store(false, Ordering::Release);
        self.wait_queue().wake_all();
    }

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
            ..Default::default()
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

    fn try_lock(self) -> Option<LockedCachePage> {
        try_lock_page(&self).map(|wait_queue| LockedCachePage::new(self, wait_queue))
    }

    fn try_lock_guard(&self) -> Option<LockedCachePageGuard<'_>> {
        try_lock_page(self).map(|wait_queue| LockedCachePage::new(self, wait_queue))
    }

    fn lock(self) -> LockedCachePage {
        let wait_queue = lock_page(&self);
        LockedCachePage::new(self, wait_queue)
    }

    fn lock_guard(&self) -> LockedCachePageGuard<'_> {
        if let Some(locked_page) = self.try_lock_guard() {
            return locked_page;
        }

        let wait_queue = lock_page(self);
        LockedCachePage::new(self, wait_queue)
    }

    fn ensure_init(&self, init_fn: impl FnOnce(LockedCachePage) -> Result<()>) -> Result<()> {
        // Fast path: if the page is already initialized, return immediately without waiting.
        if !self.is_uninit() {
            return Ok(());
        }

        let locked_page = self.lock_guard();
        // Check again after acquiring the lock to avoid duplicate initialization.
        if !locked_page.is_uninit() {
            return Ok(());
        }

        init_fn(locked_page.into_owned())
    }
}

/// A locked cache page that owns its page handle by default.
///
/// The locked page has the exclusive right to perform critical
/// state transitions (e.g., preparing for I/O).
///
/// Use [`LockedCachePageGuard`] when the lock should borrow an existing
/// [`CachePage`] instead of taking ownership of it.
pub struct LockedCachePage<PageRef: Borrow<CachePage> = CachePage> {
    page: Option<PageRef>,
    wait_queue: &'static WaitQueue,
}

/// A borrowed guard for a locked cache page.
pub type LockedCachePageGuard<'a> = LockedCachePage<&'a CachePage>;

impl<PageRef: Borrow<CachePage>> Debug for LockedCachePage<PageRef> {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("LockedCachePage")
            .field("page", &self.page.as_ref().map(|page| page.borrow()))
            .finish()
    }
}

impl<PageRef: Borrow<CachePage>> LockedCachePage<PageRef> {
    fn new(page: PageRef, wait_queue: &'static WaitQueue) -> Self {
        Self {
            page: Some(page),
            wait_queue,
        }
    }

    fn page(&self) -> &CachePage {
        self.page.as_ref().expect("page already taken").borrow()
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
    pub fn wait_until_finish_writing_back(&self) {
        self.wait_queue
            .wait_until(|| (!self.is_writing_back()).then_some(()));
    }

    /// Checks if the page is currently being written back to storage.
    pub(super) fn is_writing_back(&self) -> bool {
        self.metadata().is_writing_back.load(Ordering::Acquire)
    }
}

impl LockedCachePage<CachePage> {
    /// Unlocks the page and returns the underlying cache page.
    pub fn unlock(mut self) -> CachePage {
        let page = self.page.take().expect("page already taken");
        unlock_page(&page, self.wait_queue);
        page
    }
}

impl LockedCachePage<&CachePage> {
    /// Converts a borrowed locked page into an owned locked page.
    pub(super) fn into_owned(mut self) -> LockedCachePage {
        let page = self.page.take().expect("page already taken").clone();
        LockedCachePage::new(page, self.wait_queue)
    }
}

impl<PageRef: Borrow<CachePage>> Deref for LockedCachePage<PageRef> {
    type Target = CachePage;

    fn deref(&self) -> &Self::Target {
        self.page()
    }
}

impl<PageRef: Borrow<CachePage>> Drop for LockedCachePage<PageRef> {
    fn drop(&mut self) {
        if let Some(page) = &self.page {
            unlock_page(page.borrow(), self.wait_queue);
        }
    }
}

fn unlock_page(page: &CachePage, wait_queue: &'static WaitQueue) {
    page.metadata().lock.store(false, Ordering::Release);
    wait_queue.wake_all();
}

fn try_lock_page(page: &CachePage) -> Option<&'static WaitQueue> {
    let wait_queue = page.wait_queue();
    page.metadata()
        .lock
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
        .then_some(wait_queue)
}

fn lock_page(page: &CachePage) -> &'static WaitQueue {
    let wait_queue = page.wait_queue();
    wait_queue.wait_until(|| {
        page.metadata()
            .lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
    });
    wait_queue
}
