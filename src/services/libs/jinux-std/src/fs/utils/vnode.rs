use crate::prelude::*;

use super::{Inode, PageCacheManager};
use crate::rights::Rights;
use crate::vm::vmo::{Vmo, VmoFlags, VmoOptions};

/// VFS-level representation of an inode
pub struct Vnode {
    inode: Arc<dyn Inode>,
    pages: Vmo,
}

impl Vnode {
    pub fn new(inode: Arc<dyn Inode>) -> Result<Self> {
        let page_cache_manager = Arc::new(PageCacheManager::new(&Arc::downgrade(&inode)));
        let pages = VmoOptions::<Rights>::new(inode.metadata().size)
            .flags(VmoFlags::RESIZABLE)
            .pager(page_cache_manager)
            .alloc()?;
        Ok(Self { inode, pages })
    }

    pub fn dup(&self) -> Result<Self> {
        let pages = self.pages.dup()?;
        Ok(Self {
            inode: self.inode.clone(),
            pages,
        })
    }

    pub fn pages(&self) -> &Vmo {
        &self.pages
    }

    pub fn inode(&self) -> &Arc<dyn Inode> {
        &self.inode
    }
}
