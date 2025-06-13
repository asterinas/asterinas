// SPDX-License-Identifier: MPL-2.0

use alloc::{
    string::ToString,
    sync::{Arc, Weak},
};

use ostd::sync::RwLock;

use crate::{
    error::Errno,
    fs::{
        cgroupfs::systree_node::{CgroupNormalNode, CgroupUnifiedNode},
        path::{is_dot, is_dotdot},
        utils::{
            FileSystem, InnerNode, Inode, InodeMode, InodeType, KernelFsInode, Metadata, NAME_MAX,
        },
    },
    return_errno, return_errno_with_message, Result,
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

    fn create(&self, name: &str, _type_: InodeType, _mode: InodeMode) -> Result<Arc<dyn Inode>> {
        if name.len() > NAME_MAX {
            return_errno!(Errno::ENAMETOOLONG);
        }

        let InnerNode::Branch(branch_node) = &self.inner_node else {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        };

        if branch_node.child(name).is_some() {
            return_errno_with_message!(Errno::EEXIST, "entry exists");
        }

        let new_child = CgroupNormalNode::new(name.to_string().into());
        if branch_node.is_root() {
            let tree_node = branch_node
                .as_any()
                .downcast_ref::<CgroupUnifiedNode>()
                .unwrap();
            tree_node.add_child(new_child.clone())?;
        } else {
            let tree_node = branch_node
                .as_any()
                .downcast_ref::<CgroupNormalNode>()
                .unwrap();
            tree_node.add_child(new_child.clone())?;
        };

        let new_inode = Self::new_branch_dir(InnerNode::Branch(new_child), self.parent.clone());
        Ok(new_inode)
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        if is_dot(name) {
            return_errno_with_message!(Errno::EINVAL, "rmdir on .");
        }
        if is_dotdot(name) {
            return_errno_with_message!(Errno::ENOTEMPTY, "rmdir on ..");
        }

        let InnerNode::Branch(branch_node) = self.inner_node() else {
            return_errno_with_message!(Errno::ENOTDIR, "current node is not a branch node");
        };

        let target_node = branch_node
            .child(name)
            .ok_or(crate::Error::new(Errno::ENOENT))?;
        if target_node.cast_to_branch().unwrap().count_children() != 0 {
            return_errno_with_message!(
                Errno::ENOTEMPTY,
                "only an empty cgroup hierarchy can be removed"
            );
        }

        let target_cgroup_node = Arc::downcast::<CgroupNormalNode>(target_node).unwrap();
        if target_cgroup_node.have_processes() {
            return_errno_with_message!(Errno::EBUSY, "the cgroup hierarchy still has processes");
        }

        if branch_node.is_root() {
            branch_node
                .as_any()
                .downcast_ref::<CgroupUnifiedNode>()
                .unwrap()
                .remove_child(name);
        } else {
            branch_node
                .as_any()
                .downcast_ref::<CgroupNormalNode>()
                .unwrap()
                .remove_child(name);
        }

        Ok(())
    }
}
