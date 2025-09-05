// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use crate::fs::cgroupfs::fs::CgroupFsType;

mod fs;
mod inode;
mod systree_node;

// This method should be called during kernel file system initialization,
// _after_ `aster_systree::init`.
pub(super) fn init() {
    let cgroupfs_type = Arc::new(CgroupFsType);
    super::registry::register(cgroupfs_type).unwrap();
}
