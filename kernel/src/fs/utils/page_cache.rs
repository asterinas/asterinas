// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use core::{iter, ops::Range};

use align_ext::AlignExt;
use aster_block::bio::{BioStatus, BioWaiter};
use aster_rights::Full;
use lru::LruCache;
use ostd::mm::{Frame, FrameAllocOptions, VmIo};

use crate::{
    prelude::*,
    vm::vmo::{get_page_idx_range, Pager, Vmo, VmoFlags, VmoOptions},
};

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
        pages: &mut MutexGuard<LruCache<usize, Page>>,
    ) -> Result<()> {
        if matches!(self.waiter.wait(), Some(BioStatus::Complete)) {
            let Some(window) = &self.ra_window else {
                return_errno!(Errno::EINVAL)
            };
            for idx in window.readahead_range() {
                if let Some(page) = pages.get_mut(&idx) {
                    page.set_state(PageState::UpToDate);
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
        pages: &mut MutexGuard<LruCache<usize, Page>>,
        backend: Arc<dyn PageCacheBackend>,
    ) -> Result<()> {
        let Some(window) = &self.ra_window else {
            return_errno!(Errno::EINVAL)
        };
        for async_idx in window.readahead_range() {
            let mut async_page = Page::alloc()?;
            let pg_waiter = backend.read_page_async(async_idx, async_page.frame())?;
            if pg_waiter.nreqs() > 0 {
                self.waiter.concat(pg_waiter);
            } else {
                // Some backends (e.g. RamFS) do not issue requests, but fill the page directly.
                async_page.set_state(PageState::UpToDate);
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
    pages: Mutex<LruCache<usize, Page>>,
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
                if *page.state() == PageState::Dirty && idx < backend_npages {
                    let waiter = backend.write_page_async(idx, page.frame())?;
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
            page.set_state(PageState::UpToDate);
        }
        Ok(())
    }

    fn ondemand_readahead(&self, idx: usize) -> Result<Frame> {
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
            if let PageState::Uninit = page.state() {
                // Cond 2: We should wait for the previous readahead.
                // If there is no previous readahead, an error must have occurred somewhere.
                assert!(ra_state.request_number() != 0);
                ra_state.wait_for_prev_readahead(&mut pages)?;
                pages.get(&idx).unwrap().frame().clone()
            } else {
                // Cond 1.
                page.frame().clone()
            }
        } else {
            // Cond 3.
            // Conducts the sync read operation.
            let page = if idx < backend.npages() {
                let mut page = Page::alloc()?;
                backend.read_page(idx, page.frame())?;
                page.set_state(PageState::UpToDate);
                page
            } else {
                Page::alloc_zero()?
            };
            let frame = page.frame().clone();
            pages.put(idx, page);
            frame
        };
        if ra_state.should_readahead(idx, backend.npages()) {
            ra_state.setup_window(idx, backend.npages());
            ra_state.conduct_readahead(&mut pages, backend)?;
        }
        ra_state.set_prev_page(idx);
        Ok(frame)
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
    fn commit_page(&self, idx: usize) -> Result<Frame> {
        self.ondemand_readahead(idx)
    }

    fn update_page(&self, idx: usize) -> Result<()> {
        let mut pages = self.pages.lock();
        if let Some(page) = pages.get_mut(&idx) {
            page.set_state(PageState::Dirty);
        } else {
            warn!("The page {} is not in page cache", idx);
        }

        Ok(())
    }

    fn decommit_page(&self, idx: usize) -> Result<()> {
        let page_result = self.pages.lock().pop(&idx);
        if let Some(page) = page_result {
            if let PageState::Dirty = page.state() {
                let Some(backend) = self.backend.upgrade() else {
                    return Ok(());
                };
                if idx < backend.npages() {
                    backend.write_page(idx, page.frame())?;
                }
            }
        }

        Ok(())
    }

    fn commit_overwrite(&self, idx: usize) -> Result<Frame> {
        if let Some(page) = self.pages.lock().get(&idx) {
            return Ok(page.frame.clone());
        }

        let page = Page::alloc_zero()?;
        Ok(self.pages.lock().get_or_insert(idx, || page).frame.clone())
    }
}

#[derive(Debug)]
struct Page {
    frame: Frame,
    state: PageState,
}

impl Page {
    pub fn alloc() -> Result<Self> {
        let frame = FrameAllocOptions::new(1).uninit(true).alloc_single()?;
        Ok(Self {
            frame,
            state: PageState::Uninit,
        })
    }

    pub fn alloc_zero() -> Result<Self> {
        let frame = FrameAllocOptions::new(1).alloc_single()?;
        Ok(Self {
            frame,
            state: PageState::Dirty,
        })
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn state(&self) -> &PageState {
        &self.state
    }

    pub fn set_state(&mut self, new_state: PageState) {
        self.state = new_state;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PageState {
    /// `Uninit` indicates a new allocated page which content has not been initialized.
    /// The page is available to write, not available to read.
    Uninit,
    /// `UpToDate` indicates a page which content is consistent with corresponding disk content.
    /// The page is available to read and write.
    UpToDate,
    /// `Dirty` indicates a page which content has been updated and not written back to underlying disk.
    /// The page is available to read and write.
    Dirty,
}

/// This trait represents the backend for the page cache.
pub trait PageCacheBackend: Sync + Send {
    /// Reads a page from the backend asynchronously.
    fn read_page_async(&self, idx: usize, frame: &Frame) -> Result<BioWaiter>;
    /// Writes a page to the backend asynchronously.
    fn write_page_async(&self, idx: usize, frame: &Frame) -> Result<BioWaiter>;
    /// Returns the number of pages in the backend.
    fn npages(&self) -> usize;
}

impl dyn PageCacheBackend {
    /// Reads a page from the backend synchronously.
    fn read_page(&self, idx: usize, frame: &Frame) -> Result<()> {
        let waiter = self.read_page_async(idx, frame)?;
        match waiter.wait() {
            Some(BioStatus::Complete) => Ok(()),
            _ => return_errno!(Errno::EIO),
        }
    }
    /// Writes a page to the backend synchronously.
    fn write_page(&self, idx: usize, frame: &Frame) -> Result<()> {
        let waiter = self.write_page_async(idx, frame)?;
        match waiter.wait() {
            Some(BioStatus::Complete) => Ok(()),
            _ => return_errno!(Errno::EIO),
        }
    }
}
