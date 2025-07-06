// SPDX-License-Identifier: MPL-2.0

mod fs;
mod inode;
#[cfg(ktest)]
mod test;
mod utils;

use alloc::sync::Arc;

use aster_systree::SysMode;
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

    let fs_dir = BasicBranchNode::new("fs".into(), SysMode::DEFAULT_RW_MODE);

    systree
        .root()
        .add_child(fs_dir.clone())
        .expect("Failed to add fs directory to SysTree");
    FS_SYS_TREE_NODE.call_once(|| fs_dir);

    // Ensure systree is initialized first. This should be handled by the kernel's init order.
    SYSFS_SINGLETON.call_once(SysFs::new);
}

static FS_SYS_TREE_NODE: Once<Arc<BasicBranchNode>> = Once::new();

/// Returns the node in `SysTree` that represents the `/fs` directory.
pub fn fs_dir() -> &'static Arc<BasicBranchNode> {
    FS_SYS_TREE_NODE.get().unwrap()
}
