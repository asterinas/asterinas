use crate::prelude::*;
use crate::vm::vmo::{Pager, Vmo, VmoFlags, VmoOptions};

use core::ops::Range;
use ext2::traits::{PageCache as PageCacheTrait, PageIoObj};
use jinux_frame::vm::{VmAllocOptions, VmFrame, VmFrameVec};
use jinux_rights::Full;
use lru::LruCache;

pub struct PageCache {
    pages: Vmo<Full>,
    manager: Arc<PageCacheManager>,
}

impl PageCache {
    pub fn new(len: usize, backed_inode: Weak<dyn PageIoObj>) -> Result<Self> {
        let manager = Arc::new(PageCacheManager::new(backed_inode));
        let pages = VmoOptions::<Full>::new(len)
            .flags(VmoFlags::RESIZABLE)
            .pager(manager.clone())
            .alloc()?;
        Ok(Self { pages, manager })
    }

    pub fn pages(&self) -> &Vmo<Full> {
        &self.pages
    }

    /// Evict the data within a specified range from the page cache and persist
    /// them to the disk.
    pub fn evict_range(&self, range: Range<usize>) {
        // TODO: Implement this method.
        warn!("pagecache: evict_range is not implemented");
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

impl PageCacheTrait for PageCache {
    fn new(size: usize, inode: Weak<dyn PageIoObj>) -> Box<dyn PageCacheTrait> {
        Box::new(PageCache::new(size, inode).unwrap())
    }

    fn resize(&self, new_size: usize) -> ext2::error::Result<()> {
        self.pages.resize(new_size).unwrap();
        Ok(())
    }

    fn pages(&self) -> Box<dyn MemStorage> {
        Box::new(self.pages.dup().unwrap())
    }

    fn evict_range(&self, range: Range<usize>) -> ext2::error::Result<()> {
        self.evict_range(range);
        Ok(())
    }
}

struct PageCacheManager {
    pages: Mutex<LruCache<usize, Page>>,
    backed_inode: Weak<dyn PageIoObj>,
}

impl PageCacheManager {
    pub fn new(backed_inode: Weak<dyn PageIoObj>) -> Self {
        Self {
            pages: Mutex::new(LruCache::unbounded()),
            backed_inode,
        }
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
            page.frame()
        } else {
            let page = if offset < self.backed_inode.upgrade().unwrap().len() {
                let mut page = Page::alloc_zero()?;
                self.backed_inode
                    .upgrade()
                    .unwrap()
                    .read_page(page_idx, &page.frame())?;
                page.set_state(PageState::UpToDate);
                page
            } else {
                Page::alloc_zero()?
            };
            let frame = page.frame();
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
                    .write_page(page_idx, &page.frame())?
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
        let frame = {
            let mut vm_alloc_option = VmAllocOptions::new(1);
            vm_alloc_option.uninit(true);
            let mut frames = VmFrameVec::allocate(&vm_alloc_option)?;
            frames.pop().unwrap()
        };
        Ok(Self {
            frame,
            state: PageState::Uninit,
        })
    }

    pub fn alloc_zero() -> Result<Self> {
        let frame = {
            let vm_alloc_option = VmAllocOptions::new(1);
            let mut frames = VmFrameVec::allocate(&vm_alloc_option)?;
            frames.pop().unwrap()
        };
        Ok(Self {
            frame,
            state: PageState::Dirty,
        })
    }

    pub fn frame(&self) -> VmFrame {
        self.frame.clone()
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
