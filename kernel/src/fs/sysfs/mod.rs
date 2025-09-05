// SPDX-License-Identifier: MPL-2.0

mod fs;
mod inode;
#[cfg(ktest)]
mod test;

pub use aster_systree::primary_tree as systree_singleton;
use fs::SysFsType;

// This method should be called during kernel file system initialization,
// _after_ `aster_systree::init`.
pub fn init() {
    super::registry::register(&SysFsType).unwrap();
}
