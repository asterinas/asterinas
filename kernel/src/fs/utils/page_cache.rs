// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

use core::{
    iter,
    num::NonZero,
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
};

use align_ext::AlignExt;
use aster_block::bio::{BioStatus, BioWaiter};
use aster_rights::Full;
use lru::LruCache;
use ostd::{
    impl_untyped_frame_meta_for,
    mm::{Frame, FrameAllocOptions, UFrame, VmIo},
    prelude::Paddr,
};

use crate::{
    prelude::*,
    vm::{
        mem_total,
        vmo::{get_page_idx_range, Pager, Vmo, VmoFlags, VmoOptions, Vmo_},
    },
};

/// This part implements the global reclaim mechanism for the page cache.
///
/// Structure:
/// - PageCacheReclaimer: The global page cache reclaimer, using active and inactive LruCache to
///   manage the cache pages.
/// - ReclaimPolicy: Control reclamation and demotion through thresholds, supporting flexible expansion.
///
/// Reclaimation design:
/// - The reclaimer is devided into two parts: active and inactive LruCache.
/// - Pages that are first accessed will be added to the inactive LruCache (fn add_cache_page).
/// - Pages that are accessed again will be promoted.
///   For pages in the inactive LruCache, fn promote_from_inactive will be called to promote them to the active LruCache.
///   For pages in the active LruCache, no operation will be performed because the LruCache will naturally maintain the LRU order.
///
/// - When the number of active pages exceeds the threshold (demote_threshold, default 30% of the capacity),
///   demotion will be triggered to move the least recently used page in the active LruCache to the inactive LruCache (fn demote).
///
/// - When the total number of pages in the reclaimer exceeds the threshold (reclaim_threshold, default 90% of the capacity), reclamation will be triggered.
///   Reclamation priorizes reclaiming from the inactive LruCache. If there is no suitable page, try the active LruCache.
///
/// - Currently, only not-mmapped pages will be considered for reclamation.
/// - When reclaiming, it will determine whether the page is mmapped, whether it is dirty, and will
///   write it back to the backend if necessary.
///
/// Threshold and magic number description:
/// - The threshold is multiplied by 100 to avoid floating-point operations.
///   To indicate this, the threshold variables are named as *_threshold_times100.
///
/// - MEM_MB_THRESHOLD_FOR_RECLAIM: the threshold is used to force reclamation in low memory scenarios.
/// - DEFAULT_POWER_PAGE_SIZE: The page size is 2^DEFAULT_POWER_PAGE_SIZE by default.
///
const MEM_MB_THRESHOLD_FOR_RECLAIM: usize = 1024 * 1024 * 1024; // 1GB
const DEFAULT_POWER_PAGE_SIZE: usize = 12; // 4KB
pub struct PageCacheReclaimer {
    inner: ReclaimerInner,
    policy: Arc<dyn ReclaimPolicy + Send + Sync>,
}

struct ReclaimerInner {
    /// Maximum number of pages in the PageCacheReclaimer
    capacity: usize,
    /// PageCache: inactive LruCache + active LruCache
    inactive: LruCache<Paddr, CachePage>,
    active: LruCache<Paddr, CachePage>,
}

trait ReclaimPolicy: Send + Sync {
    fn should_reclaim(&self, reclaimer: &ReclaimerInner) -> bool;
    fn should_demote(&self, reclaimer: &ReclaimerInner) -> bool;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

struct ThresholdReclaimPolicy {
    /// Page number threshold(x100) of the active LruCache for triggering demotion
    demote_threshold_times100: usize,
    /// Page number threshold(x100) of the PageCache for triggering reclamation
    reclaim_threshold_times100: usize,
}

impl ReclaimPolicy for ThresholdReclaimPolicy {
    fn should_reclaim(&self, reclaimer: &ReclaimerInner) -> bool {
        let active_len = reclaimer.active.len();
        let inactive_len = reclaimer.inactive.len();
        let total_size = active_len + inactive_len;
        mem_total() < MEM_MB_THRESHOLD_FOR_RECLAIM
            || total_size * 100 >= self.reclaim_threshold_times100
    }

    fn should_demote(&self, reclaimer: &ReclaimerInner) -> bool {
        reclaimer.active.len() * 100 >= self.demote_threshold_times100
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl PageCacheReclaimer {
    pub fn new(capacity: usize) -> Self {
        let reclaimer_capacity =
            NonZero::new(capacity).expect("LRU capacity must be greater than zero");
        Self {
            inner: ReclaimerInner {
                capacity,
                inactive: LruCache::new(reclaimer_capacity),
                active: LruCache::new(reclaimer_capacity),
            },
            policy: Arc::new(ThresholdReclaimPolicy {
                demote_threshold_times100: usize::MAX,
                reclaim_threshold_times100: usize::MAX,
            }),
        }
    }

    /// Set demote_threshold
    pub fn set_demote_threshold_times100(&mut self, threshold_times100: usize) {
        if let Some(policy) = Arc::get_mut(&mut self.policy) {
            if let Some(tp) = policy.as_any_mut().downcast_mut::<ThresholdReclaimPolicy>() {
                tp.demote_threshold_times100 = threshold_times100;
            }
        }
    }

    /// Set reclaim_threshold
    pub fn set_reclaim_threshold_times100(&mut self, threshold_times100: usize) {
        if let Some(policy) = Arc::get_mut(&mut self.policy) {
            if let Some(tp) = policy.as_any_mut().downcast_mut::<ThresholdReclaimPolicy>() {
                tp.reclaim_threshold_times100 = threshold_times100;
            }
        }
    }

    pub fn add_cache_page(&mut self, cache_page: CachePage) -> Result<()> {
        while self.policy.should_reclaim(&self.inner) {
            self.do_reclaim()?;
        }

        // Add page to the inactive LruCache
        self.inner
            .inactive
            .push(cache_page.start_paddr(), cache_page);
        Ok(())
    }

    /// Move the lru page of the active LruCache to the inactive LruCache
    pub fn demote(&mut self) -> Result<()> {
        if self.inner.active.is_empty() {
            return Ok(());
        }

        let (demoted_addr, demoted) = self.inner.active.pop_lru().unwrap();
        self.inner.inactive.push(demoted_addr, demoted);
        Ok(())
    }

    /// Move the page from the inactive LruCache to the active LruCache
    pub fn promote_from_inactive(&mut self, cache_page: CachePage) -> Result<()> {
        let Some(target) = self.inner.inactive.pop(&cache_page.start_paddr()) else {
            return_errno!(Errno::ENOENT);
        };

        // Demote to ensure the active LruCache is not full
        while self.policy.should_demote(&self.inner) {
            self.demote()?;
        }

        self.inner.active.push(cache_page.start_paddr(), cache_page);
        Ok(())
    }

    pub fn exists_in_inactive(&self, cache_page: &CachePage) -> bool {
        self.inner.inactive.contains(&cache_page.start_paddr())
    }

    pub fn exists_in_active(&self, cache_page: &CachePage) -> bool {
        self.inner.active.contains(&cache_page.start_paddr())
    }

    pub fn remove_from_reclaimer(&mut self, page: &CachePage) {
        if self.exists_in_active(page) {
            self.inner.active.pop(&page.start_paddr());
        } else if self.exists_in_inactive(page) {
            self.inner.inactive.pop(&page.start_paddr());
        } else {
            warn!("The page is not in PageCacheReclaimer!");
        }
    }

    fn should_reclaim(&self) -> bool {
        self.policy.should_reclaim(&self.inner)
    }

    /// Reclaim one page from the inactive LruCache
    fn do_reclaim(&mut self) -> Result<()> {
        let mut reclaimed;

        // First, try to reclaim a page from the inactive LruCache
        let keys: Vec<_> = self.inner.inactive.iter().map(|(addr, _)| *addr).collect();
        for addr in keys {
            if let Some(page) = self.inner.inactive.pop(&addr) {
                let reclaimed = self.try_reclaim(&page);
                if reclaimed {
                    return Ok(());
                } else {
                    self.inner.inactive.push(addr, page);
                }
            }
        }

        // If there is no suitable page for reclamation in the inactive LruCache,
        // try to reclaim one from the active LruCache.
        while !self.inner.active.is_empty() {
            let (candidate_addr, candidate_page) = self.inner.active.pop_lru().unwrap();
            reclaimed = self.try_reclaim(&candidate_page);
            if reclaimed {
                return Ok(());
            } else {
                // The page is not suitable for reclaiming, move it to the inactive LruCache.
                self.inner.inactive.push(candidate_addr, candidate_page);
            }
        }
        // Error: no suitable page for reclaiming
        return_errno!(Errno::ENOENT);
    }

    fn try_reclaim(&mut self, page: &CachePage) -> bool {
        // Not mmapped page
        if !page.metadata().is_mmapped.load(Ordering::Relaxed) {
            let reverse_map = page.metadata().reverse_map.read().clone();
            // Reverse_map should not be none
            if let Some(map) = reverse_map {
                // Delete the reference to the victim page in Vmo_.pager.pages
                let pager = map.pager();
                if let Some(page_cache_manager) =
                    (&pager as &dyn Any).downcast_ref::<PageCacheManager>()
                {
                    // (If dirty) write the page to the disk and mark the page as UpToDate
                    let _ = page_cache_manager
                        .evict_range(page.start_paddr()..page.start_paddr() + PAGE_SIZE);
                    // Delete the page from the Vmo_.pager.pages
                    let _ = page_cache_manager
                        .discard_range(page.start_paddr()..page.start_paddr() + PAGE_SIZE);
                }
                // Delete the victim page in the PageCacheReclaimer.inner
                self.inner.inactive.pop(&page.start_paddr());
                return true;
            }
        }
        false
    }
}

static PAGE_CACHE_RECLAIMER: spin::Once<Mutex<PageCacheReclaimer>> = spin::Once::new();

pub fn get_page_cache_reclaimer() -> &'static Mutex<PageCacheReclaimer> {
    if !PAGE_CACHE_RECLAIMER.is_completed() {
        let mut reclaimer = PageCacheReclaimer::new(mem_total() >> DEFAULT_POWER_PAGE_SIZE);

        // Set thresholds for the reclaimer
        reclaimer.set_demote_threshold_times100(reclaimer.inner.capacity * 30);
        reclaimer.set_reclaim_threshold_times100(reclaimer.inner.capacity * 90);

        PAGE_CACHE_RECLAIMER.call_once(|| Mutex::new(reclaimer));
    }
    PAGE_CACHE_RECLAIMER.get().unwrap()
}

pub struct PageCache {
    pages: Vmo<Full>,
    manager: Arc<PageCacheManager>,
}

impl PageCache {
    /// Creates an empty size page cache associated with a new backend.
    pub fn new(backend: Weak<dyn PageCacheBackend>) -> Result<Self> {
        let manager = Arc::new(PageCacheManager::new(backend));
        let pages = VmoOptions::<Full>::new(0)
            .flags(VmoFlags::RESIZABLE)
            .pager(manager.clone())
            .alloc()?;
        Ok(Self { pages, manager })
    }

    /// Creates a page cache associated with an existing backend.
    ///
    /// The `capacity` is the initial cache size required by the backend.
    /// This size usually corresponds to the size of the backend.
    pub fn with_capacity(capacity: usize, backend: Weak<dyn PageCacheBackend>) -> Result<Self> {
        let manager = Arc::new(PageCacheManager::new(backend));
        let pages = VmoOptions::<Full>::new(capacity)
            .flags(VmoFlags::RESIZABLE)
            .pager(manager.clone())
            .alloc()?;
        Ok(Self { pages, manager })
    }

    /// Returns the Vmo object.
    // TODO: The capability is too high，restrict it to eliminate the possibility of misuse.
    //       For example, the `resize` api should be forbidded.
    pub fn pages(&self) -> &Vmo<Full> {
        &self.pages
    }

    /// Evict the data within a specified range from the page cache and persist
    /// them to the backend.
    pub fn evict_range(&self, range: Range<usize>) -> Result<()> {
        self.manager.evict_range(range)
    }

    /// Evict the data within a specified range from the page cache without persisting
    /// them to the backend.
    pub fn discard_range(&self, range: Range<usize>) {
        self.manager.discard_range(range)
    }

    /// Returns the backend.
    pub fn backend(&self) -> Arc<dyn PageCacheBackend> {
        self.manager.backend()
    }

    /// Resizes the current page cache to a target size.
    pub fn resize(&self, new_size: usize) -> Result<()> {
        // If the new size is smaller and not page-aligned,
        // first zero the gap between the new size and the
        // next page boundary (or the old size), if such a gap exists.
        let old_size = self.pages.size();
        if old_size > new_size && new_size % PAGE_SIZE != 0 {
            let gap_size = old_size.min(new_size.align_up(PAGE_SIZE)) - new_size;
            if gap_size > 0 {
                self.fill_zeros(new_size..new_size + gap_size)?;
            }
        }
        self.pages.resize(new_size)
    }

    /// Fill the specified range with zeros in the page cache.
    pub fn fill_zeros(&self, range: Range<usize>) -> Result<()> {
        if range.is_empty() {
            return Ok(());
        }
        let (start, end) = (range.start, range.end);

        // Write zeros to the first partial page if any
        let first_page_end = start.align_up(PAGE_SIZE);
        if first_page_end > start {
            let zero_len = first_page_end.min(end) - start;
            self.pages()
                .write_vals(start, iter::repeat_n(&0, zero_len), 0)?;
        }

        // Write zeros to the last partial page if any
        let last_page_start = end.align_down(PAGE_SIZE);
        if last_page_start < end && last_page_start >= start {
            let zero_len = end - last_page_start;
            self.pages()
                .write_vals(last_page_start, iter::repeat_n(&0, zero_len), 0)?;
        }

        for offset in (first_page_end..last_page_start).step_by(PAGE_SIZE) {
            self.pages()
                .write_vals(offset, iter::repeat_n(&0, PAGE_SIZE), 0)?;
        }
        Ok(())
    }
}

impl Drop for PageCache {
    fn drop(&mut self) {
        // TODO:
        // The default destruction procedure exhibits slow performance.
        // In contrast, resizing the `VMO` to zero greatly accelerates the process.
        // We need to find out the underlying cause of this discrepancy.
        let _ = self.pages.resize(0);
    }
}

impl Debug for PageCache {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("PageCache")
            .field("size", &self.pages.size())
            .field("mamager", &self.manager)
            .finish()
    }
}

struct ReadaheadWindow {
    /// The window.
    window: Range<usize>,
    /// Look ahead position in the current window, where the readahead is triggered.
    /// TODO: We set the `lookahead_index` to the start of the window for now.
    /// This should be adjustable by the user.
    lookahead_index: usize,
}

impl ReadaheadWindow {
    pub fn new(window: Range<usize>) -> Self {
        let lookahead_index = window.start;
        Self {
            window,
            lookahead_index,
        }
    }

    /// Gets the next readahead window.
    /// Most of the time, we push the window forward and double its size.
    ///
    /// The `max_size` is the maximum size of the window.
    /// The `max_page` is the total page number of the file, and the window should not
    /// exceed the scope of the file.
    pub fn next(&self, max_size: usize, max_page: usize) -> Self {
        let new_start = self.window.end;
        let cur_size = self.window.end - self.window.start;
        let new_size = (cur_size * 2).min(max_size).min(max_page - new_start);
        Self {
            window: new_start..(new_start + new_size),
            lookahead_index: new_start,
        }
    }

    pub fn lookahead_index(&self) -> usize {
        self.lookahead_index
    }

    pub fn readahead_index(&self) -> usize {
        self.window.end
    }

    pub fn readahead_range(&self) -> Range<usize> {
        self.window.clone()
    }
}

struct ReadaheadState {
    /// Current readahead window.
    ra_window: Option<ReadaheadWindow>,
    /// Maximum window size.
    max_size: usize,
    /// The last page visited, used to determine sequential I/O.
    prev_page: Option<usize>,
    /// Readahead requests waiter.
    waiter: BioWaiter,
}

impl ReadaheadState {
    const INIT_WINDOW_SIZE: usize = 4;
    const DEFAULT_MAX_SIZE: usize = 32;

    pub fn new() -> Self {
        Self {
            ra_window: None,
            max_size: Self::DEFAULT_MAX_SIZE,
            prev_page: None,
            waiter: BioWaiter::new(),
        }
    }

    /// Sets the maximum readahead window size.
    pub fn set_max_window_size(&mut self, size: usize) {
        self.max_size = size;
    }

    fn is_sequential(&self, idx: usize) -> bool {
        if let Some(prev) = self.prev_page {
            idx == prev || idx == prev + 1
        } else {
            false
        }
    }

    /// The number of bio requests in waiter.
    /// This number will be zero if there are no previous readahead.
    pub fn request_number(&self) -> usize {
        self.waiter.nreqs()
    }

    /// Checks for the previous readahead.
    /// Returns true if the previous readahead has been completed.
    pub fn prev_readahead_is_completed(&self) -> bool {
        let nreqs = self.request_number();
        if nreqs == 0 {
            return false;
        }

        for i in 0..nreqs {
            if self.waiter.status(i) == BioStatus::Submit {
                return false;
            }
        }
        true
    }

    /// Waits for the previous readahead.
    pub fn wait_for_prev_readahead(
        &mut self,
        pages: &mut MutexGuard<LruCache<usize, CachePage>>,
    ) -> Result<()> {
        if matches!(self.waiter.wait(), Some(BioStatus::Complete)) {
            let Some(window) = &self.ra_window else {
                return_errno!(Errno::EINVAL)
            };
            for idx in window.readahead_range() {
                if let Some(page) = pages.get_mut(&idx) {
                    page.store_state(PageState::UpToDate);
                }
            }
            self.waiter.clear();
        } else {
            return_errno!(Errno::EIO)
        }

        Ok(())
    }

    /// Determines whether a new readahead should be performed.
    /// We only consider readahead for sequential I/O now.
    /// There should be at most one in-progress readahead.
    pub fn should_readahead(&self, idx: usize, max_page: usize) -> bool {
        if self.request_number() == 0 && self.is_sequential(idx) {
            if let Some(cur_window) = &self.ra_window {
                let trigger_readahead =
                    idx == cur_window.lookahead_index() || idx == cur_window.readahead_index();
                let next_window_exist = cur_window.readahead_range().end < max_page;
                trigger_readahead && next_window_exist
            } else {
                let new_window_start = idx + 1;
                new_window_start < max_page
            }
        } else {
            false
        }
    }

    /// Setup the new readahead window.
    pub fn setup_window(&mut self, idx: usize, max_page: usize) {
        let new_window = if let Some(cur_window) = &self.ra_window {
            cur_window.next(self.max_size, max_page)
        } else {
            let start_idx = idx + 1;
            let init_size = Self::INIT_WINDOW_SIZE.min(self.max_size);
            let end_idx = (start_idx + init_size).min(max_page);
            ReadaheadWindow::new(start_idx..end_idx)
        };
        self.ra_window = Some(new_window);
    }

    /// Conducts the new readahead.
    /// Sends the relevant read request and sets the relevant page in the page cache to `Uninit`.
    pub fn conduct_readahead(
        &mut self,
        pages: &mut MutexGuard<LruCache<usize, CachePage>>,
        backend: Arc<dyn PageCacheBackend>,
    ) -> Result<()> {
        let Some(window) = &self.ra_window else {
            return_errno!(Errno::EINVAL)
        };
        for async_idx in window.readahead_range() {
            let mut async_page = CachePage::alloc_uninit()?;
            let pg_waiter = backend.read_page_async(async_idx, &async_page)?;
            if pg_waiter.nreqs() > 0 {
                self.waiter.concat(pg_waiter);
            } else {
                // Some backends (e.g. RamFS) do not issue requests, but fill the page directly.
                async_page.store_state(PageState::UpToDate);
            }
            pages.put(async_idx, async_page);
        }
        Ok(())
    }

    /// Sets the last page visited.
    pub fn set_prev_page(&mut self, idx: usize) {
        self.prev_page = Some(idx);
    }
}

struct PageCacheManager {
    pages: Mutex<LruCache<usize, CachePage>>,
    backend: Weak<dyn PageCacheBackend>,
    ra_state: Mutex<ReadaheadState>,
}

impl PageCacheManager {
    pub fn new(backend: Weak<dyn PageCacheBackend>) -> Self {
        Self {
            pages: Mutex::new(LruCache::unbounded()),
            backend,
            ra_state: Mutex::new(ReadaheadState::new()),
        }
    }

    pub fn backend(&self) -> Arc<dyn PageCacheBackend> {
        self.backend.upgrade().unwrap()
    }

    // Discard pages without writing them back to disk.
    pub fn discard_range(&self, range: Range<usize>) {
        let page_idx_range = get_page_idx_range(&range);
        let mut pages = self.pages.lock();
        for idx in page_idx_range {
            // First, delete the page from the PageCacheReclaimer
            if let Some(page) = pages.peek(&idx) {
                get_page_cache_reclaimer()
                    .lock()
                    .remove_from_reclaimer(page);
            }
            // Then delete the page from the pages of PageCacheManager
            pages.pop(&idx);
        }
    }

    pub fn evict_range(&self, range: Range<usize>) -> Result<()> {
        let page_idx_range = get_page_idx_range(&range);

        let mut bio_waiter = BioWaiter::new();
        let mut pages = self.pages.lock();
        let backend = self.backend();
        let backend_npages = backend.npages();
        for idx in page_idx_range.start..page_idx_range.end {
            if let Some(page) = pages.peek(&idx) {
                if page.load_state() == PageState::Dirty && idx < backend_npages {
                    let waiter = backend.write_page_async(idx, page)?;
                    bio_waiter.concat(waiter);
                }
            }
        }

        if !matches!(bio_waiter.wait(), Some(BioStatus::Complete)) {
            // Do not allow partial failure
            return_errno!(Errno::EIO);
        }

        for (_, page) in pages
            .iter_mut()
            .filter(|(idx, _)| page_idx_range.contains(*idx))
        {
            page.store_state(PageState::UpToDate);
        }
        Ok(())
    }

    fn ondemand_readahead(&self, idx: usize) -> Result<UFrame> {
        let mut pages = self.pages.lock();
        let mut ra_state = self.ra_state.lock();
        let backend = self.backend();
        // Checks for the previous readahead.
        if ra_state.prev_readahead_is_completed() {
            ra_state.wait_for_prev_readahead(&mut pages)?;
        }
        // There are three possible conditions that could be encountered upon reaching here.
        // 1. The requested page is ready for read in page cache.
        // 2. The requested page is in previous readahead range, not ready for now.
        // 3. The requested page is on disk, need a sync read operation here.
        let frame = if let Some(page) = pages.get(&idx) {
            // Cond 1 & 2.
            if let PageState::Uninit = page.load_state() {
                // Cond 2: We should wait for the previous readahead.
                // If there is no previous readahead, an error must have occurred somewhere.
                assert!(ra_state.request_number() != 0);
                ra_state.wait_for_prev_readahead(&mut pages)?;
                pages.get(&idx).unwrap().clone()
            } else {
                // Cond 1.
                page.clone()
            }
        } else {
            // Cond 3.
            // Conducts the sync read operation.
            let page = if idx < backend.npages() {
                let mut page = CachePage::alloc_uninit()?;
                backend.read_page(idx, &page)?;
                page.store_state(PageState::UpToDate);
                page
            } else {
                CachePage::alloc_zero(PageState::Uninit)?
            };
            let frame = page.clone();
            pages.put(idx, page);
            // Add page to the PageCacheReclaimer.
            get_page_cache_reclaimer()
                .lock()
                .add_cache_page(frame.clone())
                .unwrap();
            frame
        };
        if ra_state.should_readahead(idx, backend.npages()) {
            ra_state.setup_window(idx, backend.npages());
            ra_state.conduct_readahead(&mut pages, backend)?;
        }
        ra_state.set_prev_page(idx);
        Ok(frame.into())
    }
}

impl Debug for PageCacheManager {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("PageCacheManager")
            .field("pages", &self.pages.lock())
            .finish()
    }
}

impl Pager for PageCacheManager {
    fn commit_page(&self, idx: usize) -> Result<UFrame> {
        self.ondemand_readahead(idx)
    }

    fn update_page(&self, idx: usize) -> Result<()> {
        let mut pages = self.pages.lock();
        if let Some(page) = pages.get_mut(&idx) {
            page.store_state(PageState::Dirty);
        } else {
            warn!("The page {} is not in page cache", idx);
        }

        Ok(())
    }

    fn decommit_page(&self, idx: usize) -> Result<()> {
        // First, delete the page from the PageCacheManager
        let page_result = self.pages.lock().pop(&idx);

        if let Some(page) = page_result {
            // Second, delete the page from the PageCacheReclaimer
            get_page_cache_reclaimer()
                .lock()
                .remove_from_reclaimer(&page);
            // Third, if dirty, write back
            if let PageState::Dirty = page.load_state() {
                let Some(backend) = self.backend.upgrade() else {
                    return Ok(());
                };
                if idx < backend.npages() {
                    backend.write_page(idx, &page)?;
                }
            }
        }

        Ok(())
    }

    fn commit_overwrite(&self, idx: usize) -> Result<UFrame> {
        if let Some(page) = self.pages.lock().get(&idx) {
            return Ok(page.clone().into());
        }

        let page = CachePage::alloc_uninit()?;
        let page_tmp = self.pages.lock().get_or_insert(idx, || page).clone();
        // Add page to the PageCacheReclaimer.
        get_page_cache_reclaimer()
            .lock()
            .add_cache_page(page_tmp.clone())
            .unwrap();
        Ok(page_tmp.into())
    }

    /// PageCacheReclaimer-related: promote a page from the inactive LRUCache to the active LRUCache
    fn g_page_cache_promote(&self, idx: usize) -> Result<()> {
        let mut pages = self.pages.lock();
        if let Some(page) = pages.get(&idx) {
            let mut reclaimer = get_page_cache_reclaimer().lock();
            if reclaimer.exists_in_inactive(page) {
                reclaimer.promote_from_inactive(page.clone())?;
            }
        } else {
            warn!("The page {} is not in page cache", idx);
        }
        Ok(())
    }
}

/// A page in the page cache.
pub type CachePage = Frame<CachePageMeta>;

/// Metadata for a page in the page cache.
#[derive(Debug)]
pub struct CachePageMeta {
    pub state: AtomicPageState,
    pub reverse_map: RwLock<Option<Arc<Vmo_>>>,
    pub is_mmapped: AtomicBool,
}

impl_untyped_frame_meta_for!(CachePageMeta);

pub trait CachePageExt {
    /// Gets the metadata associated with the cache page.
    fn metadata(&self) -> &CachePageMeta;

    /// Allocates a new cache page which content and state are uninitialized.
    fn alloc_uninit() -> Result<CachePage> {
        let meta = CachePageMeta {
            state: AtomicPageState::new(PageState::Uninit),
            reverse_map: RwLock::new(None),
            is_mmapped: AtomicBool::new(false),
        };
        let page = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_frame_with(meta)?;
        Ok(page)
    }

    /// Allocates a new zeroed cache page with the wanted state.
    fn alloc_zero(state: PageState) -> Result<CachePage> {
        let meta = CachePageMeta {
            state: AtomicPageState::new(state),
            reverse_map: RwLock::new(None),
            is_mmapped: AtomicBool::new(false),
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

    /// Stores a new state for the cache page.
    fn store_state(&mut self, new_state: PageState) {
        self.metadata().state.store(new_state, Ordering::Relaxed);
    }
}

impl CachePageExt for CachePage {
    fn metadata(&self) -> &CachePageMeta {
        self.meta()
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
    fn read_page_async(&self, idx: usize, frame: &CachePage) -> Result<BioWaiter>;
    /// Writes a page to the backend asynchronously.
    fn write_page_async(&self, idx: usize, frame: &CachePage) -> Result<BioWaiter>;
    /// Returns the number of pages in the backend.
    fn npages(&self) -> usize;
}

impl dyn PageCacheBackend {
    /// Reads a page from the backend synchronously.
    fn read_page(&self, idx: usize, frame: &CachePage) -> Result<()> {
        let waiter = self.read_page_async(idx, frame)?;
        match waiter.wait() {
            Some(BioStatus::Complete) => Ok(()),
            _ => return_errno!(Errno::EIO),
        }
    }
    /// Writes a page to the backend synchronously.
    fn write_page(&self, idx: usize, frame: &CachePage) -> Result<()> {
        let waiter = self.write_page_async(idx, frame)?;
        match waiter.wait() {
            Some(BioStatus::Complete) => Ok(()),
            _ => return_errno!(Errno::EIO),
        }
    }
}
