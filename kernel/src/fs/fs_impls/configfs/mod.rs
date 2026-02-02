// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::{EmptyNode, SysBranchNode};
use systree_node::ConfigRootNode;

use crate::{fs::configfs::fs::ConfigFsType, prelude::*};

mod fs;
mod inode;
mod systree_node;
#[cfg(ktest)]
mod test;

// This method should be called during kernel file system initialization,
// _after_ `aster_systree::init`.
pub(super) fn init() {
    let config_kernel_sysnode = EmptyNode::new("config".into());
    super::sysfs::register_kernel_sysnode(config_kernel_sysnode).unwrap();

    super::registry::register(&ConfigFsType).unwrap();
}

/// Registers a subsystem `SysTree` node under the root node of [`ConfigFs`].
///
/// If a subsystem with the same name has already been registered,
/// this function returns an error.
///
/// [`ConfigFs`]: fs::ConfigFs
#[cfg_attr(not(ktest), expect(dead_code))]
pub fn register_subsystem(subsystem: Arc<dyn SysBranchNode>) -> Result<()> {
    ConfigRootNode::singleton().add_child(subsystem)?;

    Ok(())
}

/// Unregisters a subsystem from the root node of [`ConfigFs`] by its name.
///
/// If no subsystem with the given name exists, this function returns an error.
///
/// [`ConfigFs`]: fs::ConfigFs
#[expect(dead_code)]
pub fn unregister_subsystem(name: &str) -> Result<()> {
    ConfigRootNode::singleton().remove_child(name)?;

    Ok(())
}

#[cfg(ktest)]
pub fn init_for_ktest() {
    aster_systree::init_for_ktest();
    super::registry::init();
    super::sysfs::init();
    init();
}
