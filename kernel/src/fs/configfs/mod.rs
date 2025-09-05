// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::{EmptyNode, SysBranchNode};
use fs::ConfigFs;
use spin::Once;
use systree_node::ConfigRootNode;

use crate::fs::configfs::fs::ConfigFsType;

mod fs;
mod inode;
mod systree_node;
#[cfg(ktest)]
mod test;

static CONFIG_SINGLETON: Once<Arc<ConfigFs>> = Once::new();

/// Returns a reference to the global [`ConfigFs`] instance.
///
/// # Panics
///
/// if the instance is not initialized, this function will panic.
pub fn singleton() -> &'static Arc<ConfigFs> {
    CONFIG_SINGLETON.get().expect("ConfigFs is not initialized")
}

/// Initializes the [`ConfigFs`] singleton and adds the config directory to the `SysTree`.
pub(super) fn init() {
    let config_kernel_sysnode = EmptyNode::new("config".into());
    super::sysfs::register_kernel_sysnode(config_kernel_sysnode).unwrap();

    let config_root = ConfigRootNode::new();
    CONFIG_SINGLETON.call_once(|| ConfigFs::new(config_root));

    let config_fs_type = Arc::new(ConfigFsType);
    super::registry::register(config_fs_type).unwrap();
}

/// Registers a subsystem `SysTree` node under the root node of [`ConfigFs`].
///
/// # Panics
///
/// If the [`ConfigFs`] has not been initialized, or a subsystem with the same name has
/// already been registered, this function will panic.
pub fn register_subsystem(subsystem: Arc<dyn SysBranchNode>) {
    CONFIG_SINGLETON
        .get()
        .unwrap()
        .systree_root()
        .add_child(subsystem)
        .expect("a subsystem with the same name has already been registered");
}

#[cfg(ktest)]
pub fn init_for_ktest() {
    aster_systree::init_for_ktest();
    super::registry::init();
    super::sysfs::init();
    init();
}
