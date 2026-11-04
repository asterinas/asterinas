// SPDX-License-Identifier: MPL-2.0

mod fs;
mod inode;
#[cfg(ktest)]
mod test;

use fs::SysFsType;

use crate::fs::vfs::registry;

// This method should be called during kernel file system initialization,
// _after_ `aster_systree::init`.
pub(crate) fn init() {
    registry::register(&SysFsType).unwrap();
}
