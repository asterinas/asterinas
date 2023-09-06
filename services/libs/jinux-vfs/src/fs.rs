use crate::inode::Inode;
use crate::metadata::SuperBlock;
use crate::prelude::*;

pub trait FileSystem: Any + Sync + Send {
    fn sync(&self) -> Result<()>;

    fn root_inode(&self) -> Arc<dyn Inode>;

    fn sb(&self) -> SuperBlock;

    fn flags(&self) -> FsFlags;
}

impl dyn FileSystem {
    pub fn downcast_ref<T: FileSystem>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }
}

impl Debug for dyn FileSystem {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("FileSystem")
            .field("super_block", &self.sb())
            .field("flags", &self.flags())
            .finish()
    }
}

bitflags! {
    pub struct FsFlags: u32 {
        /// Disable page cache.
        const NO_PAGECACHE = 1 << 0;
        /// Dentry cannot be evicted.
        const DENTRY_UNEVICTABLE = 1 << 1;
    }
}
