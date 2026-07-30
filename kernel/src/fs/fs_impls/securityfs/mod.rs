// SPDX-License-Identifier: MPL-2.0

//! The securityfs pseudo filesystem.
//!
//! [`fs::SecurityFs`] is the singleton VFS filesystem.
//! [`systree_node::SecurityRootNode`] collects top-level [`aster_systree`] nodes from active LSM modules.
//! [`inode::SecurityFsInode`] adapts that system tree to VFS inodes.

use aster_systree::EmptyNode;

use crate::fs::securityfs::fs::SecurityFsType;

mod fs;
mod inode;
mod systree_node;

pub(super) fn init() {
    let security_kernel_sysnode = EmptyNode::new("security".into());
    super::sysfs::register_kernel_sysnode(security_kernel_sysnode).unwrap();

    crate::fs::vfs::registry::register(&SecurityFsType).unwrap();
}
