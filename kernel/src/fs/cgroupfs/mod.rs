// SPDX-License-Identifier: MPL-2.0

use fs::CgroupFsType;
pub use systree_node::{CgroupMembership, CgroupNode, CgroupSysNode};

mod controller;
mod fs;
mod inode;
mod systree_node;

// This method should be called during kernel file system initialization,
// _after_ `aster_systree::init`.
pub(super) fn init() {
    super::registry::register(&CgroupFsType).unwrap();
}
