// SPDX-License-Identifier: MPL-2.0

mod fs;
mod inode;
mod kernel;
#[cfg(ktest)]
mod test;

use alloc::sync::Arc;

pub use aster_systree::primary_tree as systree_singleton;
use aster_systree::{SysNode, SysObj};
use spin::Once;

pub use self::fs::SysFs;
use crate::{fs::sysfs::fs::SysFsType, prelude::*};

static SYSFS_SINGLETON: Once<Arc<SysFs>> = Once::new();

/// Returns a reference to the global [`SysFs`] instance.
///
/// # Panics
///
/// if the instance is not initialized, this function will panic.
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

    kernel::init();
}

/// Registers a new kernel `SysNode`.
pub fn register_kernel_sysnode(config_obj: Arc<dyn SysNode>) -> Result<()> {
    kernel::register(config_obj)
}

/// Unregisters a kernel `SysNode`.
#[expect(dead_code)]
pub fn unregister_kernel_sysnode(name: &str) -> Result<Arc<dyn SysObj>> {
    kernel::unregister(name)
}
