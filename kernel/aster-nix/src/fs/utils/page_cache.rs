// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use aster_block::bio::{BioStatus, BioWaiter};
use aster_frame::vm::{VmAllocOptions, VmFrame};
use aster_rights::Full;
use lru::LruCache;

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

struct PageCacheManager {
    pages: Mutex<LruCache<usize, Page>>,
    backend: Weak<dyn PageCacheBackend>,
}

impl PageCacheManager {
    pub fn new(backend: Weak<dyn PageCacheBackend>) -> Self {
        Self {
            pages: Mutex::new(LruCache::unbounded()),
            backend,
        }
    }

    pub fn backend(&self) -> Arc<dyn PageCacheBackend> {
        self.backend.upgrade().unwrap()
    }

    // Discard pages without writing them back to disk.
    pub fn discard_range(&self, range: Range<usize>) {
        let page_idx_range = get_page_idx_range(&range);
        for idx in page_idx_range {
            self.pages.lock().pop(&idx);
        }
    }

    pub fn evict_range(&self, range: Range<usize>) -> Result<()> {
        let page_idx_range = get_page_idx_range(&range);

        //TODO: When there are many pages, we should submit them in batches of folios rather than all at once.
        let mut indices_and_waiters: Vec<(usize, BioWaiter)> = Vec::new();

        for idx in page_idx_range {
            if let Some(page) = self.pages.lock().get_mut(&idx) {
                if let PageState::Dirty = page.state() {
                    let backend = self.backend();
                    if idx < backend.npages() {
                        indices_and_waiters.push((idx, backend.write_page(idx, page.frame())?));
                    }
                }
            }
        }

        for (idx, waiter) in indices_and_waiters.iter() {
            if matches!(waiter.wait(), Some(BioStatus::Complete)) {
                if let Some(page) = self.pages.lock().get_mut(idx) {
                    page.set_state(PageState::UpToDate)
                }
            } else {
                // TODO: We may need an error handler here.
                return_errno!(Errno::EIO)
            }
        }

        Ok(())
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
    fn commit_page(&self, idx: usize) -> Result<VmFrame> {
        if let Some(page) = self.pages.lock().get(&idx) {
            return Ok(page.frame.clone());
        }

        //Multiple threads may commit the same page, but the result is ok.
        let backend = self.backend();
        let page = if idx < backend.npages() {
            let mut page = Page::alloc()?;
            backend.read_page_sync(idx, page.frame())?;
            page.set_state(PageState::UpToDate);

            page
        } else {
            Page::alloc_zero()?
        };
        let frame = page.frame().clone();
        self.pages.lock().put(idx, page);
        Ok(frame)
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
                    backend.write_page_sync(idx, page.frame())?;
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
struct Page {
    frame: VmFrame,
    state: PageState,
}

impl Page {
    pub fn alloc() -> Result<Self> {
        let frame = VmAllocOptions::new(1).uninit(true).alloc_single()?;
        Ok(Self {
            frame,
            state: PageState::Uninit,
        })
    }

    pub fn alloc_zero() -> Result<Self> {
        let frame = VmAllocOptions::new(1).alloc_single()?;
        Ok(Self {
            frame,
            state: PageState::Dirty,
        })
    }

    pub fn frame(&self) -> &VmFrame {
        &self.frame
    }

    pub fn state(&self) -> &PageState {
        &self.state
    }

    pub fn set_state(&mut self, new_state: PageState) {
        self.state = new_state;
    }
}

#[derive(Debug)]
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
    fn read_page(&self, idx: usize, frame: &VmFrame) -> Result<BioWaiter>;
    /// Writes a page to the backend asynchronously.
    fn write_page(&self, idx: usize, frame: &VmFrame) -> Result<BioWaiter>;
    /// Returns the number of pages in the backend.
    fn npages(&self) -> usize;
}

impl dyn PageCacheBackend {
    /// Reads a page from the backend synchronously.
    fn read_page_sync(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        let waiter = self.read_page(idx, frame)?;
        match waiter.wait() {
            Some(BioStatus::Complete) => Ok(()),
            _ => return_errno!(Errno::EIO),
        }
    }
    /// Writes a page to the backend synchronously.
    fn write_page_sync(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        let waiter = self.write_page(idx, frame)?;
        match waiter.wait() {
            Some(BioStatus::Complete) => Ok(()),
            _ => return_errno!(Errno::EIO),
        }
    }
}
