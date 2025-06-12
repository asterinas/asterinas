// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use fs::CgroupFs;
pub use inode::CgroupInode;
use spin::Once;
pub use systree_node::CgroupNormalNode;
use systree_node::CgroupUnifiedNode;

mod fs;
mod inode;
mod systree_node;

static CGROUP_ROOT_NODE: Once<Arc<CgroupUnifiedNode>> = Once::new();
static CGROUP_SINGLETON: Once<Arc<CgroupFs>> = Once::new();

/// Returns a reference to the global CgroupFs instance. Panics if not initialized.
pub fn singleton() -> &'static Arc<CgroupFs> {
    CGROUP_SINGLETON.get().expect("CgroupFs is not initialized")
}

/// Returns a reference to the root node of the cgroup unified hierarchy.
pub fn root_node() -> &'static Arc<CgroupUnifiedNode> {
    CGROUP_ROOT_NODE
        .get()
        .expect("cgroup root node is not initialized")
}

/// Initializes the CgroupFs singleton.
/// Ensures that the singleton is created by calling it.
/// Should be called during kernel filesystem initialization, *after* aster_systree::init().
pub fn init() {
    let fs_node = super::sysfs::fs_dir();
    let cgroup_dir = CgroupUnifiedNode::new();

    fs_node
        .add_child(cgroup_dir.clone())
        .expect("Failed to add cgroup directory to SysTree");

    // Ensure systree is initialized first. This should be handled by the kernel's init order.
    CGROUP_SINGLETON.call_once(|| CgroupFs::new(cgroup_dir.clone()));
    CGROUP_ROOT_NODE.call_once(|| cgroup_dir);
}
