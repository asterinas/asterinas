// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use fs::CgroupFs;
pub use inode::CgroupInode;
use spin::Once;

use crate::fs::cgroupfs::systree_node::CgroupSystem;

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

    super::manager::singleton()
        .register(cgroup_root.clone())
        .expect("cannot register cgroup factory to fs manager");

    // Ensure systree is initialized first. This should be handled by the kernel's init order.
    CGROUP_SINGLETON.call_once(|| CgroupFs::new(cgroup_root));
}
