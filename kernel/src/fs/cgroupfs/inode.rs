// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};

use ostd::sync::RwLock;

use crate::{
    fs::utils::{
        systree_inode::{SysTreeInodeTy, SysTreeNodeKind},
        FileSystem, Inode, InodeMode, Metadata,
    },
    Result,
};

/// An inode abstraction used in the cgroup file system.
pub struct CgroupInode {
    /// The corresponding node in the SysTree.
    node_kind: SysTreeNodeKind,
    /// The metadata of this inode.
    metadata: Metadata,
    /// The file mode (permissions) of this inode, protected by a lock.
    mode: RwLock<InodeMode>,
    /// Weak reference to the parent inode.
    parent: Weak<CgroupInode>,
    /// Weak self-reference for cyclic data structures.
    this: Weak<CgroupInode>,
}

impl SysTreeInodeTy for CgroupInode {
    fn new_arc(
        node_kind: SysTreeNodeKind,
        metadata: Metadata,
        mode: InodeMode,
        parent: Weak<Self>,
    ) -> Arc<Self>
    where
        Self: Sized,
    {
        Arc::new_cyclic(|this| Self {
            node_kind,
            metadata,
            mode: RwLock::new(mode),
            parent,
            this: this.clone(),
        })
    }

    fn node_kind(&self) -> &SysTreeNodeKind {
        &self.node_kind
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
