use super::Inode;
use crate::prelude::*;
use crate::vm::vmo::Pager;
use jinux_frame::vm::{VmAllocOptions, VmFrame, VmFrameVec};
use lru::LruCache;

pub struct PageCacheManager {
    pages: Mutex<LruCache<usize, Page>>,
    backed_inode: Weak<dyn Inode>,
}

impl PageCacheManager {
    pub fn new(inode: &Weak<dyn Inode>) -> Self {
        Self {
            pages: Mutex::new(LruCache::unbounded()),
            backed_inode: inode.clone(),
        }
    }
}

impl Pager for PageCacheManager {
    fn commit_page(&self, offset: usize) -> Result<VmFrame> {
        let page_idx = offset / PAGE_SIZE;
        let mut pages = self.pages.lock();
        let frame = if let Some(page) = pages.get(&page_idx) {
            page.frame()
        } else {
            let page = if offset < self.backed_inode.upgrade().unwrap().metadata().size {
                let mut page = Page::alloc()?;
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
            match page.state() {
                PageState::Dirty => self
                    .backed_inode
                    .upgrade()
                    .unwrap()
                    .write_page(page_idx, &page.frame())?,
                _ => (),
            }
        } else {
            warn!("page {} is not in page cache, do nothing", page_idx);
        }
        Ok(())
    }
}

struct Page {
    frame: VmFrame,
    state: PageState,
}

impl Page {
    pub fn alloc() -> Result<Self> {
        let frame = {
            let vm_alloc_option = VmAllocOptions::new(1);
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
            frames.zero();
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
