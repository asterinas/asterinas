// SPDX-License-Identifier: MPL-2.0

//! The securityfs filesystem.
//!
//! Security modules register files and directories through
//! [`crate::security::securityfs`]. All mounts share the same tree.

mod fs;
mod inode;
mod node;

use aster_systree::{EmptyNode, SysObj};
use fs::SecurityFsType;
use node::{SecurityDirectory, SecurityRootNode};

use crate::{fs::vfs::registry, prelude::*, security::securityfs::SecurityFsDirectory};

pub(super) fn init() {
    SecurityRootNode::singleton();
    super::sysfs::register_kernel_sysnode(EmptyNode::new("security".into())).unwrap();
    registry::register(&SecurityFsType).unwrap();
}

pub(crate) fn register_directory(
    directory: &SecurityFsDirectory,
    children: &[Arc<dyn SysObj>],
) -> Result<()> {
    let node = SecurityDirectory::new(directory)?;
    for child in children {
        node.add_node(child.clone())?;
    }
    SecurityRootNode::singleton().add_child(node)?;
    Ok(())
}
