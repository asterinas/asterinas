// SPDX-License-Identifier: MPL-2.0

mod fs;
mod inode;
mod kernel;
#[cfg(ktest)]
mod test;

pub use aster_systree::primary_tree as systree_singleton;
use aster_systree::SysNode;
use fs::SysFsType;

use crate::prelude::*;

// This method should be called during kernel file system initialization,
// _after_ `aster_systree::init`.
pub fn init() {
    super::registry::register(&SysFsType).unwrap();

    kernel::init();
}

/// Registers a new kernel `SysNode`.
pub fn register_kernel_sysnode(config_obj: Arc<dyn SysNode>) -> Result<()> {
    kernel::register(config_obj)
}

/// Unregisters a kernel `SysNode`.
#[expect(dead_code)]
pub fn unregister_kernel_sysnode(name: &str) -> Result<()> {
    kernel::unregister(name)
}
