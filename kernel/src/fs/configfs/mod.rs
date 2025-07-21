// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::SysBranchNode;
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

/// Returns a reference to the global [`ConfigFs`] instance. Panics if not initialized.
pub fn singleton() -> &'static Arc<ConfigFs> {
    CONFIG_SINGLETON.get().expect("ConfigFs is not initialized")
}

/// Initializes the `ConfigFs` singleton and adds the config directory to the `SysTree`.
pub fn init() {
    let config_root = ConfigRootNode::new();
    let config_fs_type = ConfigFsType::new(config_root.clone());
    super::kernel_config::register(config_root).unwrap();
    super::registry::register(config_fs_type).unwrap();
}

/// Registers a subsystem the the configfs.
///
/// # Panics
///
/// If the configfs has not been initialized, or a subsystem with the same name has
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
    super::kernel_config::init();
    super::sysfs::init();

    let config_root = ConfigRootNode::new();
    let config_fs_type = ConfigFsType::new(config_root.clone());
    super::kernel_config::register(config_root.clone()).unwrap();
    super::registry::register(config_fs_type).unwrap();

    let configfs = ConfigFs::new(config_root);
    CONFIG_SINGLETON.call_once(|| configfs);
}
