use super::Inode;
use crate::prelude::*;
use crate::vm::vmo::{get_page_idx_range, Pager, Vmo, VmoFlags, VmoOptions};
use jinux_rights::Full;

use core::ops::Range;
use jinux_frame::vm::{VmAllocOptions, VmFrame};
use lru::LruCache;

pub struct PageCache {
    pages: Vmo<Full>,
    manager: Arc<PageCacheManager>,
}

impl PageCache {
    /// Creates an empty size page cache associated with a new inode.
    pub fn new(backed_inode: Weak<dyn Inode>) -> Result<Self> {
        let manager = Arc::new(PageCacheManager::new(backed_inode));
        let pages = VmoOptions::<Full>::new(0)
            .flags(VmoFlags::RESIZABLE)
            .pager(manager.clone())
            .alloc()?;
        Ok(Self { pages, manager })
    }

    /// Creates a page cache associated with an existing inode.
    ///
    /// The `capacity` is the initial cache size required by the inode.
    /// It is usually used the same size as the inode.
    pub fn with_capacity(capacity: usize, backed_inode: Weak<dyn Inode>) -> Result<Self> {
        let manager = Arc::new(PageCacheManager::new(backed_inode));
        let pages = VmoOptions::<Full>::new(capacity)
            .flags(VmoFlags::RESIZABLE)
            .pager(manager.clone())
            .alloc()?;
        Ok(Self { pages, manager })
    }

    /// Returns the Vmo object backed by inode.
    // TODO: The capability is too high，restrict it to eliminate the possibility of misuse.
    //       For example, the `resize` api should be forbidded.
    pub fn pages(&self) -> Vmo<Full> {
        self.pages.dup().unwrap()
    }

    /// Evict the data within a specified range from the page cache and persist
    /// them to the disk.
    pub fn evict_range(&self, range: Range<usize>) -> Result<()> {
        self.manager.evict_range(range)
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
    backed_inode: Weak<dyn Inode>,
}

impl PageCacheManager {
    pub fn new(backed_inode: Weak<dyn Inode>) -> Self {
        Self {
            pages: Mutex::new(LruCache::unbounded()),
            backed_inode,
        }
    }

    pub fn evict_range(&self, range: Range<usize>) -> Result<()> {
        let page_idx_range = get_page_idx_range(&range);
        let mut pages = self.pages.lock();
        for page_idx in page_idx_range {
            if let Some(page) = pages.get_mut(&page_idx) {
                if let PageState::Dirty = page.state() {
                    self.backed_inode
                        .upgrade()
                        .unwrap()
                        .write_page(page_idx, page.frame())?;
                    page.set_state(PageState::UpToDate);
                }
            } else {
                warn!("page {} is not in page cache, do nothing", page_idx);
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
    fn commit_page(&self, offset: usize) -> Result<VmFrame> {
        let page_idx = offset / PAGE_SIZE;
        let mut pages = self.pages.lock();
        let frame = if let Some(page) = pages.get(&page_idx) {
            page.frame().clone()
        } else {
            let backed_inode = self.backed_inode.upgrade().unwrap();
            let page = if offset < backed_inode.len() {
                let mut page = Page::alloc()?;
                backed_inode.read_page(page_idx, page.frame())?;
                page.set_state(PageState::UpToDate);
                page
            } else {
                Page::alloc_zero()?
            };
            let frame = page.frame().clone();
            pages.put(page_idx, page);
            frame
        };
        Ok(frame)
    }

    fn update_page(&self, offset: usize) -> Result<()> {
        let page_idx = offset / PAGE_SIZE;
        let mut pages = self.pages.lock();
        if let Some(page) = pages.get_mut(&page_idx) {
            page.set_state(PageState::Dirty);
        } else {
            error!("page {} is not in page cache", page_idx);
            panic!();
        }
        Ok(())
    }

    fn decommit_page(&self, offset: usize) -> Result<()> {
        let page_idx = offset / PAGE_SIZE;
        let mut pages = self.pages.lock();
        if let Some(page) = pages.pop(&page_idx) {
            if let PageState::Dirty = page.state() {
                self.backed_inode
                    .upgrade()
                    .unwrap()
                    .write_page(page_idx, page.frame())?
            }
        } else {
            warn!("page {} is not in page cache, do nothing", page_idx);
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
