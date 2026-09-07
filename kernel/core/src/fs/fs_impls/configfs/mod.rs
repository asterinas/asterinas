// SPDX-License-Identifier: MPL-2.0

//! Configfs exposes configurable kernel objects through a RAM-based file system.
//! User space can create and configure objects through the `SysTree` model.

use aster_systree::{EmptyNode, SysBranchNode};
use systree_node::ConfigRootNode;

use crate::{fs::utils::systree_fs::SingletonSysTreeFsType, prelude::*};

mod systree_node;
#[cfg(ktest)]
mod test;

const MAGIC_NUMBER: u64 = 0x62656570;
const BLOCK_SIZE: usize = 4096;
const NAME_MAX: usize = 255;

fn config_root() -> Arc<dyn SysBranchNode> {
    ConfigRootNode::singleton().clone()
}

static CONFIG_FS_TYPE: SingletonSysTreeFsType =
    SingletonSysTreeFsType::new("configfs", MAGIC_NUMBER, BLOCK_SIZE, NAME_MAX, config_root);

// This method should be called during kernel file system initialization,
// _after_ `aster_systree::init`.
pub(super) fn init() {
    let config_kernel_sysnode = EmptyNode::new("config".into());
    super::sysfs::register_kernel_sysnode(config_kernel_sysnode).unwrap();

    crate::fs::vfs::registry::register(&CONFIG_FS_TYPE).unwrap();
}

/// Registers a subsystem `SysTree` node under the Configfs root.
///
/// If a subsystem with the same name has already been registered,
/// this function returns an error.
#[cfg_attr(
    not(any(ktest, all(target_arch = "x86_64", feature = "cvm_guest"))),
    expect(dead_code)
)]
pub(crate) fn register_subsystem(subsystem: Arc<dyn SysBranchNode>) -> Result<()> {
    ConfigRootNode::singleton().add_child(subsystem)?;

    Ok(())
}

/// Unregisters a subsystem from the Configfs root by its name.
///
/// If no subsystem with the given name exists, this function returns an error.
#[expect(dead_code)]
pub(crate) fn unregister_subsystem(name: &str) -> Result<()> {
    ConfigRootNode::singleton().remove_child(name)?;

    Ok(())
}

#[cfg(ktest)]
pub(crate) fn init_for_ktest() {
    aster_systree::init_for_ktest();
    crate::fs::vfs::init();
    super::sysfs::init();
    init();
}
