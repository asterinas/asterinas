// SPDX-License-Identifier: MPL-2.0

mod fs;
mod inode;
#[cfg(ktest)]
mod test;

use alloc::sync::Arc;

pub use aster_systree::primary_tree as systree_singleton;
use spin::Once;

pub use self::fs::SysFs;
use crate::fs::sysfs::fs::SysFsType;

static SYSFS_SINGLETON: Once<Arc<SysFs>> = Once::new();

/// Returns a reference to the global [`SysFs`] instance. Panics if not initialized.
pub fn singleton() -> &'static Arc<SysFs> {
    SYSFS_SINGLETON.get().expect("SysFs not initialized")
}

/// Initializes the [`SysFs`] singleton.
///
/// Ensures that the singleton is created by calling it.
/// Should be called during kernel file system initialization, *after* aster_systree::init().
pub(super) fn init() {
    SYSFS_SINGLETON.call_once(SysFs::new);

    let sysfs_type = Arc::new(SysFsType);
    super::registry::register(sysfs_type).unwrap();
}
