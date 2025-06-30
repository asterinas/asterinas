// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::SysBranchNode;
use fs::ConfigFs;
use spin::Once;
use systree_node::ConfigRootNode;

mod fs;
mod inode;
mod systree_node;
#[cfg(ktest)]
mod test;

static CONFIG_SINGLETON: Once<Arc<ConfigFs>> = Once::new();
static CONFIG_ROOT_NODE: Once<Arc<ConfigRootNode>> = Once::new();

/// Returns a reference to the global CgroupFs instance. Panics if not initialized.
pub fn singleton() -> &'static Arc<ConfigFs> {
    CONFIG_SINGLETON.get().expect("ConfigFs is not initialized")
}

/// Initializes the `ConfigFs` singleton and adds the config directory to the `SysTree`.
pub fn init() {
    let kernel_node = super::sysfs::kernel::singleton();
    let config_dir = ConfigRootNode::new();

    kernel_node
        .add_child(config_dir.clone())
        .expect("Failed to add config directory to SysTree");

    CONFIG_ROOT_NODE.call_once(|| config_dir.clone());
    // Ensure systree is initialized first. This should be handled by the kernel's init order.
    CONFIG_SINGLETON.call_once(|| ConfigFs::new(config_dir));
}

/// Registers a subsystem the the configfs.
///
/// # Panics
///
/// If the configfs has not been initialized, or a subsystem with the same name has
/// already been registered, this function will panic.
pub fn register_subsystem(subsystem: Arc<dyn SysBranchNode>) {
    CONFIG_ROOT_NODE
        .get()
        .unwrap()
        .add_child(subsystem)
        .expect("a subsystem with the same name has already been registered");
}

#[cfg(ktest)]
pub fn init_for_ktest() {
    aster_systree::init_for_ktest();
    super::sysfs::init();
    self::init();
}
