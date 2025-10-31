// SPDX-License-Identifier: MPL-2.0

//! Device management for Asterinas.
//!
//! This crate provides the infrastructure for managing devices.
//! It includes functionality for:
//!
//! - Device ID allocation and management
//! - Device registration and unregistration
//! - Integration with the system tree (sysfs) for device discovery
//!
//! # Overview
//!
//! Devices in Asterinas are represented by the [`Device`] trait, which defines the common
//! interface that all devices must implement. Devices are classified into different types
//! using the [`DeviceType`] enum.
//!
//! Each device is identified by a [`DeviceId`], which consists of a major and minor number.
//! The crate provides mechanisms for allocating and managing these IDs through the
//! [`DeviceIdAllocator`].
//!
//! Devices are integrated into the system tree (sysfs) through their [`sysnode()`] method,
//! which returns a representation of the device in the sysfs hierarchy.
//!
//! # Examples
//!
//! Registering a device:
//!
//! ```
//! use alloc::{
//!     string::ToString,
//!     sync::{Arc, Weak},
//! };
//! use aster_device::{register_device_ids, Device, DeviceId, DeviceIdAllocator, DeviceType};
//! use aster_systree::{
//!     inherit_sys_branch_node, BranchNodeFields, Result, SysAttrSetBuilder, SysBranchNode,
//!     SysObj, SysPerms, SysStr,
//! };
//! use inherit_methods_macro::inherit_methods;
//!
//! #[derive(Debug)]
//! pub struct MyDevice {
//!     id: DeviceId,
//!     fields: BranchNodeFields<dyn SysBranchNode, Self>,
//! }
//!
//! impl Device for MyDevice {
//!     fn device_type(&self) -> DeviceType {
//!         DeviceType::Char
//!     }
//!
//!     fn device_id(&self) -> Option<DeviceId> {
//!         Some(self.id)
//!     }
//!
//!     fn sysnode(&self) -> Arc<dyn SysBranchNode> {
//!         self.weak_self().upgrade().unwrap()
//!     }
//! }
//!
//! inherit_sys_branch_node!(MyDevice, fields, {
//!    fn perms(&self) -> SysPerms {
//!        SysPerms::DEFAULT_RW_PERMS
//!    }
//! }
//!
//! #[inherit_methods(from = "self.fields")]
//! impl MyDevice {
//!     pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);
//!     pub fn weak_self(&self) -> &Weak<Self>;
//!     pub fn child(&self, name: &str) -> Option<Arc<dyn SysBranchNode>>;
//!     pub fn add_child(&self, new_child: Arc<dyn SysBranchNode>) -> Result<()>;
//!     pub fn remove_child(&self, child_name: &str) -> Result<Arc<dyn SysBranchNode>>;
//! }
//!
//! impl MyDevice {
//!     fn new(id: DeviceId, name: &str) -> Arc<Self> {
//!         let name = SysStr::from(file.name().to_string());
//!
//!         let mut builder = SysAttrSetBuilder::new();
//!         // Add common attributes.
//!         builder.add(SysStr::from("dev"), SysPerms::DEFAULT_RO_ATTR_PERMS);
//!         builder.add(SysStr::from("uevent"), SysPerms::DEFAULT_RW_ATTR_PERMS);
//!         let attrs = builder.build().expect("Failed to build attribute set");
//!
//!         Arc::new_cyclic(|weak_self| MyDevice {
//!             id,
//!             fields: BranchNodeFields::new(name, attrs, weak_self.clone()),
//!         })
//!     }
//! }
//!
//! // Register the device
//! const MY_MAJOR: u32 = 1;
//! let ida = register_device_ids(DeviceType::Char, MY_MAJOR, 0..256).unwrap();
//! let id = ida.allocate(0).unwrap();;
//! add_device(MyDevice::new(id, "my_device"));
//! ```

#![no_std]
#![deny(unsafe_code)]
#![feature(linked_list_cursors)]
#![feature(trait_upcasting)]

extern crate alloc;

mod block;
mod char;
mod id;
mod sysnode;

use alloc::{format, sync::Arc};
use core::fmt::Debug;

use aster_systree::SysBranchNode;
use component::{init_component, ComponentInitError};
pub use id::{
    register_device_ids, DeviceId, DeviceIdAllocator, DeviceType, MAJOR_BITS, MINOR_BITS,
};
use spin::Once;
use sysnode::{DevNode, DevSymlink, DevSymlinks, DevicesNode};

/// A trait representing a device.
pub trait Device: Send + Sync + Debug {
    /// Returns the type of the device.
    fn type_(&self) -> DeviceType;

    /// Returns the device ID, if the device has one.
    fn id(&self) -> Option<DeviceId>;

    /// Returns the sysfs node representing this device.
    fn sysnode(&self) -> Arc<dyn SysBranchNode>;
}

/// Adds a device to the system.
///
/// This function adds a device to the system's device management infrastructure.
/// It performs the following actions:
///
/// 1. If the device doesn't have a parent, it adds the device to the global devices node.
/// 2. It creates a symlink in the appropriate device type directory (block or char)
///    under `sys/dev` using the device's major and minor numbers.
pub fn add_device(device: Arc<dyn Device>) {
    let sysnode = device.sysnode();
    if sysnode.parent().is_none() {
        DEVICES_NODE.get().unwrap().add_child(sysnode).unwrap();
    }

    let sys_name = match device.type_() {
        DeviceType::Block => "block",
        DeviceType::Char => "char",
        DeviceType::Other => return,
    };
    let Some(id) = device.id() else {
        return;
    };
    let dev_name = format!("{}:{}", id.major(), id.minor());
    let dev_symlink = DevSymlink::new(&dev_name, &device);
    DEV_NODE
        .get()
        .unwrap()
        .child(sys_name)
        .unwrap()
        .add_child(dev_symlink)
        .unwrap();
}

/// Removes a device from the system.
///
/// This function removes a device from the system's device management infrastructure.
/// It performs the following actions:
///
/// 1. Removes the device from the global devices node.
/// 2. Removes the symlink from the appropriate device type directory (block or char)
///    under `/dev` using the device's major and minor numbers.
pub fn remove_device(device: Arc<dyn Device>) {
    let _ = DEVICES_NODE
        .get()
        .unwrap()
        .remove_child(device.sysnode().name());

    let sys_name = match device.type_() {
        DeviceType::Block => "block",
        DeviceType::Char => "char",
        DeviceType::Other => return,
    };
    let Some(id) = device.id() else {
        return;
    };
    let dev_name = format!("{}:{}", id.major(), id.minor());
    let _ = DEV_NODE
        .get()
        .unwrap()
        .child(sys_name)
        .unwrap()
        .remove_child(&dev_name);
}

/// Retrieves a device by its `DeviceType` and `DeviceID`.
///
/// This function looks up a device in the `/sys/dev`.
pub fn get_device(type_: DeviceType, id: DeviceId) -> Option<Arc<dyn Device>> {
    let dev_node = DEV_NODE.get().unwrap();

    let sys_name = match type_ {
        DeviceType::Block => "block",
        DeviceType::Char => "char",
        DeviceType::Other => return None,
    };
    let dev_symlinks = dev_node.child(sys_name)?;

    let node_name = format!("{}:{}", id.major(), id.minor());
    let symlink = dev_symlinks.child(&node_name)?;

    symlink.device()
}

/// Returns an iterator over all registered devices.
///
/// This function provides access to all devices currently in the `/sys/dev`.
pub fn all_devices() -> impl Iterator<Item = Arc<dyn Device>> {
    let dev_node = DEV_NODE.get().unwrap();
    let blocks = dev_node.child("block").unwrap().children();
    let chars = dev_node.child("char").unwrap().children();

    blocks
        .into_iter()
        .chain(chars)
        .filter_map(|node| Arc::downcast::<DevSymlink>(node).unwrap().device())
}

static DEV_NODE: Once<Arc<DevNode>> = Once::new();

static DEVICES_NODE: Once<Arc<DevicesNode>> = Once::new();

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    let sys_tree = aster_systree::primary_tree().root();

    let devices_node = DevicesNode::new();
    sys_tree.add_child(devices_node.clone()).unwrap();
    DEVICES_NODE.call_once(|| devices_node);

    let dev_node = DevNode::new();
    dev_node.add_child(DevSymlinks::new("block")).unwrap();
    dev_node.add_child(DevSymlinks::new("char")).unwrap();
    sys_tree.add_child(dev_node.clone()).unwrap();
    DEV_NODE.call_once(|| dev_node);

    Ok(())
}
