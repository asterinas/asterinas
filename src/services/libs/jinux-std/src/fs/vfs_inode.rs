use crate::prelude::*;

use super::utils::{Inode, PageCacheManager};
use crate::rights::Rights;
use crate::vm::vmo::{Vmo, VmoFlags, VmoOptions};

#[derive(Clone)]
pub struct VfsInode {
    raw_inode: Arc<dyn Inode>,
    pages: Vmo,
}

impl VfsInode {
    pub fn new(raw_inode: Arc<dyn Inode>) -> Result<Self> {
        let page_cache_manager = Arc::new(PageCacheManager::new(&Arc::downgrade(&raw_inode)));
        let pages = VmoOptions::<Rights>::new(raw_inode.metadata().size)
            .flags(VmoFlags::RESIZABLE)
            .pager(page_cache_manager)
            .alloc()?;
        Ok(Self { raw_inode, pages })
    }

    pub fn pages(&self) -> &Vmo {
        &self.pages
    }

    pub fn raw_inode(&self) -> &Arc<dyn Inode> {
        &self.raw_inode
    }
}
