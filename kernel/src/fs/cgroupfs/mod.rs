// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use fs::CgroupFs;
pub use inode::CgroupInode;
use spin::Once;

use crate::fs::cgroupfs::{fs::CgroupFsType, systree_node::CgroupSystem};

mod fs;
mod inode;
mod systree_node;

static CGROUP_SINGLETON: Once<Arc<CgroupFs>> = Once::new();

/// Returns a reference to the global CgroupFs instance. Panics if not initialized.
pub fn singleton() -> &'static Arc<CgroupFs> {
    CGROUP_SINGLETON.get().expect("CgroupFs is not initialized")
}

/// Initializes the CgroupFs singleton.
/// Ensures that the singleton is created by calling it.
/// Should be called during kernel file system initialization, *after* aster_systree::init().
pub fn init() {
    let cgroup_root = CgroupSystem::new();
    let cgroup_fs_type = CgroupFsType::new(cgroup_root);
    super::registry::register(cgroup_fs_type).unwrap();
}
