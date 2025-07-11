// SPDX-License-Identifier: MPL-2.0

mod fs;
mod inode;
pub mod kernel;
#[cfg(ktest)]
mod test;
mod utils;

use alloc::sync::Arc;

use aster_systree::{SysBranchNode, SysMode};
use spin::Once;

pub use self::{fs::SysFs, inode::SysFsInode, utils::BasicBranchNode};

static SYSFS_SINGLETON: Once<Arc<SysFs>> = Once::new();

/// Returns a reference to the global SysFs instance. Panics if not initialized.
pub fn singleton() -> &'static Arc<SysFs> {
    SYSFS_SINGLETON.get().expect("SysFs not initialized")
}

/// Initializes the SysFs singleton.
/// Ensures that the singleton is created by calling it.
/// Should be called during kernel filesystem initialization, *after* aster_systree::init().
pub fn init() {
    let systree = aster_systree::singleton();

    // Inits child nodes.
    let fs_dir = BasicBranchNode::new("fs".into(), SysMode::DEFAULT_RW_MODE);
    FS_SYS_TREE_NODE.call_once(|| fs_dir.clone());
    kernel::init();

    // Add child nodes to the SysTree.
    systree
        .root()
        .add_child(fs_dir)
        .expect("Failed to add fs directory to SysTree");
    systree
        .root()
        .add_child(kernel::singleton().clone())
        .expect("Failed to add fs directory to SysTree");

    // Ensure systree is initialized first. This should be handled by the kernel's init order.
    SYSFS_SINGLETON.call_once(SysFs::new);
}

static FS_SYS_TREE_NODE: Once<Arc<BasicBranchNode>> = Once::new();

/// Returns the node in `SysTree` that represents the `/fs` directory.
pub fn fs_dir() -> &'static Arc<BasicBranchNode> {
    FS_SYS_TREE_NODE.get().unwrap()
}
