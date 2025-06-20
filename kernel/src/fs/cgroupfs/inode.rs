// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};

use ostd::sync::RwLock;

use crate::{
    fs::utils::{FileSystem, InnerNode, Inode, InodeMode, KernelFsInode, Metadata},
    Result,
};

/// An inode abstraction used in the cgroup filesystem.
pub struct CgroupInode {
    /// The corresponding node in the SysTree.
    inner_node: InnerNode,
    /// The metadata of this inode.
    metadata: Metadata,
    /// The file mode (permissions) of this inode, protected by a lock.
    mode: RwLock<InodeMode>,
    /// Weak reference to the parent inode.
    parent: Weak<CgroupInode>,
    /// Weak self-reference for cyclic data structures.
    this: Weak<CgroupInode>,
}

impl KernelFsInode for CgroupInode {
    fn new_arc(
        inner_node: InnerNode,
        metadata: Metadata,
        mode: InodeMode,
        parent: Weak<Self>,
    ) -> Arc<Self>
    where
        Self: Sized,
    {
        Arc::new_cyclic(|this| Self {
            inner_node,
            metadata,
            mode: RwLock::new(mode),
            parent,
            this: this.clone(),
        })
    }

    fn inner_node(&self) -> &InnerNode {
        &self.inner_node
    }

    fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(*self.mode.read())
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        *self.mode.write() = mode;
        Ok(())
    }

    fn parent(&self) -> &Weak<Self> {
        &self.parent
    }

    fn this(&self) -> Arc<Self> {
        self.this.upgrade().expect("Weak ref invalid")
    }
}

impl Inode for CgroupInode {
    fn fs(&self) -> Arc<dyn FileSystem> {
        super::singleton().clone()
    }
}
