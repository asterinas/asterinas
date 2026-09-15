// SPDX-License-Identifier: MPL-2.0

//! Configfs exposes configurable kernel objects through a RAM-based file system.
//! User space can create and configure objects through the `SysTree` model.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::sync::Arc;

use aster_core::fs::systree::{SingletonSysTreeFs, register_kernel_node};
use aster_systree::{EmptyNode, Result, SysBranchNode};
use component::{ComponentInitError, init_component};

use self::systree_node::ConfigRootNode;

mod systree_node;
#[cfg(ktest)]
mod test;

const MAGIC_NUMBER: u64 = 0x62656570;
const BLOCK_SIZE: usize = 4096;
const NAME_MAX: usize = 255;

fn config_root() -> Arc<dyn SysBranchNode> {
    ConfigRootNode::singleton().clone()
}

static CONFIG_FS_TYPE: SingletonSysTreeFs =
    SingletonSysTreeFs::new("configfs", MAGIC_NUMBER, BLOCK_SIZE, NAME_MAX, config_root);

#[init_component(kthread)]
fn init() -> core::result::Result<(), ComponentInitError> {
    let config_kernel_sysnode = EmptyNode::new("config".into());
    register_kernel_node(config_kernel_sysnode).unwrap();

    CONFIG_FS_TYPE.register().unwrap();
    Ok(())
}

/// Registers a subsystem `SysTree` node under the Configfs root.
///
/// If a subsystem with the same name has already been registered,
/// this function returns an error.
pub fn register_subsystem(subsystem: Arc<dyn SysBranchNode>) -> Result<()> {
    ConfigRootNode::singleton().add_child(subsystem)?;

    Ok(())
}
