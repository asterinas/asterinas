// SPDX-License-Identifier: MPL-2.0

mod fs;
mod inode;
#[cfg(ktest)]
mod test;

use alloc::sync::Arc;

use crate::fs::sysfs::fs::SysFsType;

// This method should be called during kernel file system initialization,
// _after_ `aster_systree::init`.
pub fn init() {
    let sysfs_type = Arc::new(SysFsType);
    super::registry::register(sysfs_type).unwrap();
}
