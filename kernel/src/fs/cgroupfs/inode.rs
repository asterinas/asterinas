// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};

use ostd::sync::RwLock;

use super::fs::CgroupFs;
use crate::{
    fs::{
        cgroupfs::CgroupNode,
        path::{is_dot, is_dotdot},
        utils::{
            systree_inode::{SysTreeInodeTy, SysTreeNodeKind},
            FileSystem, Inode, InodeMode, Metadata,
        },
    },
    prelude::*,
    Result,
};

/// An inode abstraction used in the cgroup file system.
pub(super) struct CgroupInode {
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
        self.this
            .upgrade()
            .expect("invalid weak reference to `self`")
    }
}

impl Inode for CgroupInode {
    fn fs(&self) -> Arc<dyn FileSystem> {
        CgroupFs::singleton().clone()
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        if is_dot(name) {
            return_errno_with_message!(Errno::EINVAL, "rmdir on .");
        }
        if is_dotdot(name) {
            return_errno_with_message!(Errno::ENOTEMPTY, "rmdir on ..");
        }

        let SysTreeNodeKind::Branch(branch_node) = self.node_kind() else {
            return_errno_with_message!(Errno::ENOTDIR, "the current node is not a branch node");
        };

        let target_node = branch_node.child(name).ok_or(crate::Error::with_message(
            Errno::ENOENT,
            "the child node does not exist",
        ))?;

        let target_node_ref = target_node.as_any().downcast_ref::<CgroupNode>().unwrap();

        target_node_ref.try_run_if_empty(|| {
            branch_node.remove_child(name)?;
            Ok(())
        })
    }
}
