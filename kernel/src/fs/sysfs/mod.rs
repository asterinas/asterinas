// SPDX-License-Identifier: MPL-2.0

mod fs;
mod inode;

use alloc::sync::Arc;

use spin::Once; // Use spin::Once

pub use self::{fs::SysFs, inode::SysFsInode};
use crate::println; // Assuming println macro is available

// Define the singleton using spin::Once
static SYSFS_SINGLETON: Once<Arc<SysFs>> = Once::new();

/// Returns a reference to the global SysFs instance. Panics if not initialized.
pub fn singleton() -> &'static Arc<SysFs> {
    SYSFS_SINGLETON.get().expect("SysFs not initialized")
}

/// Initializes the SysFs singleton.
/// Ensures that the singleton is created by calling it.
/// Should be called during kernel filesystem initialization, *after* systree::init().
pub fn init() {
    // Ensure systree is initialized first. This should be handled by the kernel's init order.
    SYSFS_SINGLETON.call_once(|| {
        // This closure will be executed only once.
        SysFs::new()
    });
    // TODO: Log SysFs initialization
    println!("SysFs initialized.");
}
